# Agent team plan

Status: **adopted 2026-10-16 (UTC) by the user's D28 answers
(`tasks/DECISIONS-2026-10-16.md`).** This is the plan of record for *who does
the work and how it is dispatched*. The Claude side runs on Pro (D28b), so
Claude workers stop at Sonnet 5.5 and Opus 5.5 works only on A and C packages. It changes no non-negotiable in `AGENTS.md`, no human gate in
`docs/IMPLEMENTATION_PLAN.md` §1.5, no D27 answer (agents still open PRs and
never merge), and no task order in `docs/IMPLEMENTATION_PLAN.md` §§3–4. Where
it disagrees with `docs/AGENTIC_WORKFLOW_PLAN.md` §3 ("no reviewer agent"), §4
(routing table) or §9 ("two agents at most"), or with
`docs/IMPLEMENTATION_PLAN.md` §§1.3–1.4, this plan wins once adopted.

Inputs: two plans written for another project (the Living Town agent-team
plan and the Project Seed kit, supplied in `extras/` and kept out of version
control), the Claude Code documentation
for subagents, hooks, workflows and the advisor (read 2026-10-16, Claude Code
2.1.301), the OpenCode documentation (OpenCode 1.18.35), and an access test
and bake-off run on this machine on 2026-10-16 (`docs/agents/MODELS.md`).

Model IDs appear in this plan, `docs/agents/MODELS.md` and the agent
definitions because routing is their subject. Agent-log signatures keep their
existing harness form.

## Read map

| Reader | Reads | When |
|---|---|---|
| Manager | §§1–9 of this file, the handover at the top of `docs/PROJECT_STATE.md`, the in-flight rows of `tasks/BACKLOG.md` | Every session start |
| Manager | `docs/agents/MODELS.md` | When routing or escalating |
| Manager | `docs/agents/TEMPLATES.md` | When writing a brief, a review request or an agent definition |
| Worker | Its brief, `AGENTS.md`, the crate's `AGENTS.md`, and only the files the brief names | Every task |
| You | §13, gate reports, `docs/agents/MODELS.md` when costs drift | At gates |

---

## 1. The idea

The main Claude Code session, on Opus 5.5, is the **manager**. It plans waves,
writes briefs, routes each brief to the cheapest worker that can do it, reads a
script-produced summary of the result, judges what scripts cannot, opens the
PR and stops for you at the gates. It does not implement, it never reads a
worker's transcript, and it never checks by hand what a script can check.

Workers come from two pools:

- **OpenCode workers** do most implementation, test writing, scouting,
  bookkeeping and first-pass review. They run through `tools/agents/oc.py`
  (§5.1) on the OpenCode Go subscription and the free Zen models, so their
  tokens never touch the Claude quota.
- **Claude subagents** (`.claude/agents/`) do the work where Claude's quality
  is the product: architecture and contract drafts (Opus 5.5), the serialized
  S-set, reviews of core code (Sonnet 5.5), and escalations.

Scripts do every check, count and comparison. You decide, approve dependencies
and goldens, and merge.

### 1.1 What came from the plans in `extras/`

| | Ideas |
|---|---|
| **Adopted** | Manager and worker split; the five-question routing rubric; first of a kind up, the rest down; the escalation ladder; scripts before models; tool output with a verdict first, at most 40 lines and exit codes 0/1/2; brief and report templates; a stop hook that keeps a worker working while its checks fail; a hook that protects state files; pinned model IDs and effort per agent; a fresh manager session with the handover in files; questions batched with recommendations; the lesson ladder (note, rule, check) and the repeat-incident count; gate reports with usage; the emergency brake; stripping scaffolding as models improve |
| **Adapted** | Workers include OpenCode models, not only Claude tiers; merging stays with you (D27a), so `make merge` becomes "open the PR"; `make` targets become Python scripts under `tools/agents/` (Windows primary); the parallel cap comes from the disk and the S-set rather than a fixed 4; the memory system stays light because task files, `E` records, ADRs and decision batches already hold most of it; the advisor is a later trial |
| **Left out, for now** | Agent teams (several times the tokens, experimental); a YAML task registry replacing `tasks/BACKLOG.md` (it would duplicate the backlog); EARS requirements traceability (contracts already number their Behaviour items and the report maps tests to them); mutation testing (needs a new tool); the Copier template kit (it serves new projects, not this one) |

## 2. Where Claude tokens go, and the levers

Measured in this repository on 2026-10-16:

| Observation | Measurement | Lever |
|---|---|---|
| Task files hold the contract and its whole history | `tasks/T023b.md` 124 KB (about 31k tokens), `tasks/T023.md` 121 KB; blockers, decisions and completion records share the file with the contract | Workers get a generated brief (§6); the manager reads briefs and summaries, never a whole task file |
| Root `AGENTS.md` loads into every Claude and OpenCode session | 12.5 KB; the "Note" paragraph is a merged-PR log rewritten at every merge, so the shared instruction prefix changes and every agent's prompt cache restarts | D28-f: move status to `docs/PROJECT_STATE.md` |
| Crate `AGENTS.md` files have grown past their purpose | `crpg-cli` 38 KB, `crpg-sim` 30 KB, `crpg-data` 25 KB, against the workflow plan's "60-line" target | A0-8: trim each to how-to-work; the *what* lives in `docs/architecture/` |
| OpenCode already carries volume almost free | `opencode stats`: 236 sessions, 1.95 B tokens (1.88 B cache reads), $1.34 total | Make OpenCode the default implementer |
| Claude's own price ladder | Haiku 5.5 $0.10/$0.50, Sonnet 5.5 $2/$10, Opus 5.5 $4/$20, Fable 5.1 $10/$50 per MTok | Opus only where judgment is the deliverable |
| A worker transcript is large and almost none of it matters to the manager | Bake-off implementation runs: 5–42 model requests (median 10), 50k–1.7M tokens each including cache | The manager receives a ≤40-line summary from a script |

The levers, largest first:

1. **Volume off Claude.** Routine implementation, tests from a spec,
   bookkeeping and scouting go to OpenCode.
2. **Scripts before models.** Scope, gates, lints, goldens, diff stats and
   bookkeeping facts are script outputs the manager acts on by exit code.
3. **A lean manager.** Summaries instead of transcripts; a fresh session per
   wave or milestone; the handover lives in files.
4. **Lean context for everyone.** Briefs instead of task files; short
   `AGENTS.md` files; explicit read lists.
5. **Route Claude work down.** Sonnet 5.5 is the default Claude subagent,
   Haiku 5.5 takes mechanical Claude-side work, Opus 5.5 takes A and C work,
   and Fable 5.1 is used only when you ask for it.
6. **First of a kind up, the rest down.** Pay for the strong model once per
   pattern, then brief cheaper workers with that example.
7. **Cache discipline.** Pin model and effort per session and per agent file;
   keep shared prefixes (`AGENTS.md`, agent prompts) stable.

## 3. Team and roles

| Role | Runs on | Owns | Never does |
|---|---|---|---|
| **You** | — | Vision, D-batch answers, every H gate in `docs/IMPLEMENTATION_PLAN.md` §1.5, dependency and golden approvals, merges | Routine code review |
| **Manager** | Main Claude Code session, Opus 5.5 (§9 effort) | Wave plans, briefs, routing, reading summaries, judgment review, PRs, bookkeeping approval, gate reports | Implementation beyond a few lines; reading worker transcripts; merging; re-checking script results |
| **Architect** | Claude subagent `architect`, Opus 5.5 | A packages (ADR drafts) and C packages (exact contracts) that are first of a kind or in the S-set; hard debugging only when you ask (D28b) | Code in `crates/`; approving its own contract |
| **Contract drafter** | Claude subagent `implementer` (Sonnet 5.5) | C packages that follow an existing contract's pattern, from an Opus-written outline | First-of-a-kind interfaces |
| **Implementer, OpenCode** | `oc.py` with an O1 or O2 model (`docs/agents/MODELS.md`) | I packages outside the S-set, tests from a written contract | S-set work; anything the brief does not own |
| **Implementer, Claude** | Claude subagent `implementer`, Sonnet 5.5 (no Opus rung on Pro, D28b) | S-set I packages, escalations from OpenCode, tasks that need wide repository judgment | Changing a contract it was not briefed to change |
| **Reviewer** | First pass: OpenCode O1 reviewer. Core, S-set and contracts: Claude subagent `reviewer`, Sonnet 5.5 | A verdict (`ready for PR`, `fix`, `reject`) with findings by severity, correctness and the brief only | Editing the branch; style findings |
| **Test writer** ("two keys") | OpenCode O1 model from a different family than the implementer | Extra tests written from the contract alone, before reading the implementation, for core and S-set tasks | Reading the implementation first |
| **Scout** | Built-in `Explore` subagent, or OpenCode `crpg-scout` on an O2/O3 model | Answers to "where is", "what calls", "summarize this crate", in ≤20 lines | Edits |
| **Bookkeeper** | OpenCode `crpg-grunt` on an O2/O3 model, or Claude `bookkeeper` on Haiku 5.5 | `tasks/BACKLOG.md` rows, `docs/PROJECT_STATE.md`, throughput lines, agent-log entries, commit and PR text, from facts the manager supplies | Deciding what a status is |
| **Gate** | Scripts and CI | Everything in `tools/preflight.*`, the lints, `tools/agents/verify_task.py` (A0-4) | — |

## 4. Routing

### 4.1 The rubric

For each package the manager answers five questions. All low routes to O2 or
O3 (or Haiku 5.5 on the Claude side); one high routes to at least O1 (or
Sonnet 5.5); two or more high route to Claude, with Opus 5.5 for A and C
packages.

| Question | Low | High |
|---|---|---|
| Is the contract complete? | Exact Interface, Behaviour, Test and Out of scope exist and are approved | The worker must design part of it |
| Blast radius? | One crate, no public API other crates already use | Many crates code against it |
| Novel? | Follows a pattern already in the repository, named in the brief | First of its kind |
| Judgment? | Mechanical transformation of a written spec | Trade-offs, protocol failure modes, tuning |
| Cost of a subtle bug? | A test catches it | Silent drift in determinism, authority, replay or filtering |

### 4.2 Hard rules that override the rubric

1. **S-set** (`docs/IMPLEMENTATION_PLAN.md` §1.4: `crpg-contracts`, `World` or
   tick order, schema versions and migrations, the GDExtension boundary,
   workspace `Cargo.toml`, `deny.toml`, `.github/workflows/`): Claude only,
   Sonnet 5.5 or above, serialized, human-reviewed.
2. **Never-delegated list** (`docs/AGENTIC_WORKFLOW_PLAN.md` §12): any agent
   may draft; you decide and read every line.
3. **Contracts before code.** An I package starts only from an H-approved C
   package, as today. The contract is what makes the cheap tiers usable: in
   the bake-off, an exactly specified fixed-point task with 21 hidden edge
   cases was solved completely by free models as well as paid ones
   (`docs/agents/MODELS.md` §5). Without the contract, the worker is doing
   design, which routes it up (§4.1).
4. **Privacy.** The repository is public, so free models that keep or train on
   prompts may see repository content. Nothing else: no `.env`, keys, tokens,
   private paths beyond the repository, or your messages. NVIDIA's free
   endpoints are trial-only by their own terms; use them for scouting only.
5. **No model is trusted for "done".** Every worker result goes through
   `verify_task.py` and the manager's review, whatever its tier.

### 4.3 Default route by package kind

| Kind | Default | Escalate to |
|---|---|---|
| A — architecture | `architect` (Opus 5.5, high) | Fable 5.1 only on your request |
| C — contract, first of a kind or S-set | `architect` (Opus 5.5, high) | — |
| C — contract following a pattern | `implementer` (Sonnet 5.5) from an Opus outline | `architect` |
| I — outside the S-set | The strongest suitable OpenCode O1 model (`docs/agents/MODELS.md` §4) | An O1 model of another family, then Claude `implementer` (Sonnet 5.5), then the manager splits it or asks you |
| I — S-set | Claude `implementer` (Sonnet 5.5, high) | The manager splits it or asks you |
| Tests from a contract | An OpenCode O1 test writer from another family than the implementer | Claude `implementer` |
| Review, first pass | An OpenCode O1 reviewer | — |
| Review, core, S-set or contract | Claude `reviewer` (Sonnet 5.5) | You |
| G — content and data | OpenCode O2 with the `crpgc validate --json` loop | O1 |
| Bookkeeping, commit and PR text | OpenCode O2 `crpg-grunt` | Claude `bookkeeper` (Haiku 5.5) |
| Scouting | `Explore` or OpenCode `crpg-scout` | — |

**Preference (the user, 2026-10-16):** stretch tokens. Among the OpenCode
models strong enough for the job, use the intelligent free models first, then
cheap Go, then expensive Go for reviews (`docs/agents/MODELS.md` §4). Claude is
used at the manager's discretion. Claude tokens are kept for higher-level thinking:
A and C packages, the S-set (rule 1 above still holds), judgment review and
escalations. Mixed work is split so that a free worker gathers the facts and
measurements and Claude does only the design.

The O1/O2/O3 membership and the concrete model per row are in
`docs/agents/MODELS.md`, which is the only file that changes when models do.

### 4.4 Escalation ladder

1. A result that fails `verify_task.py` or review goes back once to the same
   worker with the failing output and the manager's notes.
2. A second failure moves it up a rung: O3 to O2, O2 to O1, O1 to an O1 model
   of another family, then to Claude Sonnet 5.5, the top implementer rung on
   Pro (D28b).
3. If Sonnet 5.5 fails twice, the manager splits or rewrites the package, or
   asks you.
4. A worker that reports the brief wrong or ambiguous has not failed. The
   question goes to the contract, not to a stronger model.
5. Infrastructure failures (model unavailable, quota exhausted, timeout before
   the first edit) are retried on the next model in the same tier and never
   count as attempts. A failure confined to a test with an open flake record
   (T098, QUIC case 13) counts the same way: rerun it, do not charge the
   worker.

## 5. Dispatch

### 5.1 OpenCode workers: `tools/agents/oc.py`

The manager starts every OpenCode worker through one script (A0-3) and reads
only what it prints. Interface:

```text
python tools/agents/oc.py run   --task T024 --role implement --model opencode-go/kimi-k3 [--slot 2] [--timeout 1800] [--variant high]
python tools/agents/oc.py wave  T024:implement:opencode-go/kimi-k3 T042:tests:opencode-go/glm-5.3 ...
python tools/agents/oc.py ask   --role scout --model opencode/big-pickle "Which crates call Host::pump?"
```

What `run` does, in order:

1. Prepares the slot worktree (§5.3): `task/<ID>-<slug>` from current
   `master`, untracked files cleaned, `target/` kept warm.
2. Writes the brief to the slot (`tools/agents/brief.py`, A0-5) and builds the
   prompt from the role's OpenCode agent (`.opencode/agents/crpg-<role>.md`).
3. Runs `opencode run --agent crpg-<role> -m <model> --dir <slot>
   --format json <prompt>` with these fixes, each found in testing on this
   machine:
   - **stdin closed.** `opencode run` with an open stdin pipe waits on it for
     ever; eight parallel runs all hung this way.
   - **`--dir` and `PWD` both set to the slot.** OpenCode takes its working
     directory from the inherited `PWD`, not the process cwd. Without this,
     workers read the wrong checkout or wander the drive.
   - **Starts at least 4 s apart, and a retry on `database is locked`.**
     Concurrent starts race on OpenCode's shared SQLite database; with four to
     six simultaneous starts some runs failed at once with that error.
   - **The real executable, not the shim.** On Windows `opencode` is
     `opencode.cmd`, and `cmd.exe` parses `<`, `>`, `|` and `&` inside the
     prompt; a review prompt containing `<line number>` failed every run.
     Call `opencode.exe` directly and keep the instructions in `BRIEF.md`.
   - **A timeout that kills the process tree** (`taskkill /T` on Windows),
     because a killed parent leaves the model process running.
   - **Permissions in the agent file, never `--auto`** (§5 of
     `docs/agents/TEMPLATES.md`): no external directories, no web, no push,
     merge, rebase or branch switching, no `cargo add` or `cargo update`.
   - **A retry for the reasoning stall.** Three models spent their whole
     32,000-token output allowance thinking in one step and stopped with
     finish reason `length` and no code. `oc.py` reports that as `no-edit`
     and retries once with `--variant low`.
4. Saves the event log to `.agents/runs/<ID>/<attempt>-<model>.jsonl`
   (git-ignored) with tokens, steps and cost summed from the `step_finish`
   events.
5. Runs `tools/agents/verify_task.py` on the slot (§7).
6. Prints the summary and exits 0 (verified), 1 (a check failed) or 2 (usage
   or infrastructure error, with the cause: `quota`, `402`, `timeout`,
   `no-edit`, `locked`).

The summary is at most 40 lines:

```text
FAIL T024 implement · opencode-go/kimi-k3 · attempt 1 · 41 steps · 1.2M in (1.1M cached) / 18k out · $0.00 · 9m40s
files: crates/crpg-net/src/snapshot.rs +212 −3, crates/crpg-net/tests/snapshot.rs +160
scope: ok · preflight --crate crpg-net: clippy FAILED (2 errors) · lints: ok · goldens: unchanged · tests weakened: none
--- clippy (last 12 lines) ---
...
--- worker report (first 15 lines) ---
...
next: retry with this output (attempt 2 of 2 on this rung)
```

`wave` runs several `run`s in parallel up to the slot limit and prints one
table, so a whole wave costs the manager one tool call and one table. It
refuses a wave in which two packages own the same crate: writes to one area
stay single-threaded (`docs/agents/RESEARCH-2026-10-16.md`, R7.1).

Until A0-3 lands, the manager runs one worker at a time by hand, in Git Bash,
with the same fixes, and reads only the last text event:

```bash
cd ../crpg-slots/slot-1 && PWD="$(pwd)" opencode run --agent crpg-implement \
  -m opencode-go/<model> --dir "$(pwd)" --format json "Read BRIEF.md and do it." \
  < /dev/null > ../../crpg/.agents/runs/<ID>-1.jsonl 2>&1
```

### 5.2 Claude subagents

Definitions live in `.claude/agents/` (A0-1; text in
`docs/agents/TEMPLATES.md` §4): `architect` (Opus 5.5, effort high),
`implementer` (Sonnet 5.5, effort high, `maxTurns` 80), `reviewer` (Sonnet 5.5,
effort high, read-only tools, `memory: project`), `bookkeeper` (Haiku 5.5,
effort low, `maxTurns` 30). Every one sets `disallowedTools: Agent`, so workers
never spawn workers. Model IDs are pinned in full, never aliases, and changed
only at a gate. The manager escalates by passing `model` on the spawn, which
outranks the file.

Use the built-in `Explore` agent for searches: it skips the `CLAUDE.md`
files and the git status snapshot, which keeps it cheap.

Do **not** use `isolation: worktree` for Rust work: each such worktree starts
with an empty `target/`, so every task pays a cold build of the workspace and
5–10 GB of disk (§5.3). Claude implementers work in a warm slot instead.

### 5.3 Slots, parallelism and the disk

On 2026-10-16 drive C: had 11 GB free, with a 10 GB `target/` (about half of
it stale duplicates) and the merged `crpg-t014` worktree holding 6 GB. At your
request that day the worktree, the merged local branches, the spikes' build
folders and the stale cache (`cargo clean`) were removed, leaving 43 GB free.
A built slot needs about 5–10 GB. So:

- **Two Rust build slots** (`../crpg-slots/slot-1`, `slot-2`) for
  implementation, plus unlimited non-building work (contracts, reviews,
  scouting, bookkeeping), which needs no `target/`.
- **Slots are reused, not recreated.** Each slot keeps its `target/` between
  tasks; `oc.py` resets the branch and cleans untracked files except
  `target/`. Run `cargo clean` in a slot when its `target/` passes 10 GB.
- With 50 GB or more free: four slots, the Living Town plan's cap. Below 20 GB
  free: back to one. The S-set stays serialized whatever the slot count.

### 5.4 When to use a dynamic workflow

Workflows (`.claude/workflows/`) hold a loop in a script so the manager's
context sees only the result, but the script cannot run shell commands, so an
OpenCode dispatch from a workflow costs a wrapper agent. Use them only for
Claude-side fan-outs: an adversarial review of a large diff by several
`reviewer` agents, or a codebase audit. OpenCode waves go through `oc.py wave`.
Each workflow run is its own stage; sign-off between stages stays with you.

## 6. The operating loop

Per wave:

1. **Start.** Fresh manager session. Read the read map's first row. Check CI
   on `master`. If the handover lists unresolved worker results, deal with
   them first.
2. **Plan.** Pick the wave from `docs/IMPLEMENTATION_PLAN.md` §4. List the
   packages that are `ready`, their route (§4) and slot.
3. **Brief.** `python tools/agents/brief.py <ID>` writes the skeleton from the
   task file's current contract sections; the manager adds the judgment: goal,
   out of scope, the example to copy for a follow-on task, and `recall.py`
   results. Template: `docs/agents/TEMPLATES.md` §1.
4. **Dispatch.** `oc.py wave` for OpenCode packages; background `Agent` calls
   for Claude packages. Never more than the slot limit building at once.
5. **Verify.** Already done by `oc.py`; for Claude workers, by their
   `SubagentStop` hook (A0-6). A non-zero verdict goes back per §4.4.
6. **Review.** The manager judges only what scripts cannot: does the code meet
   the contract, do the tests test the right thing, does anything exceed the
   brief? Core and S-set packages also get the Claude `reviewer`, and the
   "two keys" test writer.
7. **PR.** The manager pushes the branch and opens the PR. You merge (D27a).
8. **Bookkeeping.** The bookkeeper writes the `BACKLOG` row, the
   `PROJECT_STATE` handover and the throughput line from facts the manager
   lists. The manager checks the diff.
9. **Stop.** Update the handover in `docs/PROJECT_STATE.md` before the session
   ends or the context fills.

Per milestone, the gate (§8) adds a report and stops for you.

Questions that do not block the wave go into the next decision batch as a
draft (`tasks/DECISIONS-<date>.md` format), each with a recommendation, rather
than interrupting you.

## 7. Verification and review

### 7.1 What scripts check, never a model

`tools/agents/verify_task.py <ID> --slot <path>` (A0-4), exit codes 0/1/2,
verdict first, at most 40 lines:

| Check | Fails when |
|---|---|
| Scope | A changed file is outside the brief's owns list (the crate, its task file, its architecture doc and `AGENTS.md` when allowed) |
| Gates | `tools/preflight.sh --crate <crate>` (or `.ps1`) fails |
| Protected files | `Cargo.lock`, `deny.toml`, workspace `Cargo.toml`, `rust-toolchain.toml`, `crates/crpg-contracts/**` or `.github/workflows/**` changed without the brief's permission |
| Goldens | A golden or `.replay` file changed and the brief does not allow it |
| Test weakening | An existing test file lost an `assert`, a `#[test]` or a case, or gained `#[ignore]` |
| `unsafe` | Any new `unsafe` outside `crpg-godot` |
| Floats and maps | `determinism.py` and `deps.py` (already gates) |
| Doc signatures | A changed `.md` file has no new agent-log entry dated today (the `AGENTS.md` rule, now checked) |
| Floating `todo!()` | `todo!()` or `unimplemented!()` left in owned non-test code |

### 7.2 What the manager judges

- Each test would fail if the code were wrong.
- The code does what the contract says, not only what the existing tests
  check.
- Readings the worker had to make are listed, and none silently changes the
  contract.
- Deviations, issues and lessons in the report are handled: accepted, logged
  or turned into a follow-up.

Mutation testing would let a script answer the first question for core
crates, but it needs a new tool (`cargo-mutants`), so it waits for its own
dependency request.

### 7.3 Reviewer calibration

A reviewer told to find gaps always finds some. Reviewers report only
correctness and contract violations, with file and line. The bake-off review
task planted five defects and one decoy: 22 of the 24 models that answered
found all five with no false positives, and none flagged the decoy. That fixture
was too easy to rank the strong models, so A0-7 replaces it with real defects
from this repository's history (for example the code before the T019 and
T023d fixes), which is the review work that matters here.

## 8. Memory, handover and gates

The repository already keeps most project memory: task files, `E` records,
ADRs, decision batches, `BACKLOG.md` and `PROJECT_STATE.md`. This plan adds
little:

- **Handover.** The top of `docs/PROJECT_STATE.md` carries a three-line
  handover (what is in flight, what is next, what is blocked). A
  `SessionStart` hook with the `compact` matcher re-injects it after
  compaction (A0-6).
- **When to hand over.** A manager hands over at the first of:
  1. the end of a wave;
  2. a context of **150k tokens**: finish the package in hand and start no
     new one;
  3. a context of **200k tokens**: hand over now.

  Every turn re-reads the whole context, so cost grows with it, and so does
  the chance of missing something buried early in it. The first manager
  session (2026-10-16) reached about 346k by mid-afternoon, which is how the
  limits were set. The manager does not judge its own size: a hook,
  `tools/agents/context_meter.py` (A0-6b), reads the exact figure from the
  transcript's `usage` field after each tool call and injects a notice when
  a threshold is crossed. A notice is a rule, not a suggestion.
- **How to hand over.**
  1. Prefer a quiet point, with no worker running. A new session does not
     receive the old session's background notifications.
  2. Every run that is still going must be named in the handover with its
     slot, branch and log path. `oc.py` result records under
     `.agents/runs/` let the next manager collect it with `oc.py stats` and
     the logs, without the old session.
  3. Update the three-line handover, commit, and stop. You start the next
     session with the §12 prompt.
- **No manager hierarchy.** Sessions relay; managers do not nest.
  - Claude Code subagents cannot spawn subagents, so a sub-manager would have
    to be a separate headless session.
  - Every extra layer re-reads context and reports upward, at several times
    the tokens.
  - The layer between the manager and the workers is already scripts:
    `oc.py wave` turns a wave into one table, and `verify_task.py` does the
    checking.
- **Lessons.** `docs/agents/LESSONS.md`: one line per lesson with an ID, the
  evidence and where it was promoted. Seen twice becomes a line in an
  `AGENTS.md`; checkable by machine becomes a lint or a `verify_task.py`
  check, and then leaves the `AGENTS.md`.
- **Recall.** `tools/agents/recall.py <words>` (A0-5) searches tasks, `E`
  records, ADRs, decisions, lessons and `git log`, and the brief quotes what it
  finds.
- **Repeat incidents.** A bug whose symptom matches an earlier record counts
  as a repeat; the gate report counts them and the target is zero.

Each milestone gate report (`docs/reviews/gates/<milestone>.md`) contains:
what merged, with route, model and attempts per package; the exit criteria,
met or not, with evidence; Claude usage from `/usage` and OpenCode usage from
the run logs; planned against actual routing; repeat incidents and lessons;
model, effort and tool versions; the next milestone's wave plan; and the
questions for you.

## 9. Cost control

- **Manager model and effort.** Opus 5.5 always. Effort by session type:
  - `medium` for routine wave sessions: dispatch, read summaries, decide by
    the written rules, open PRs. Once `oc.py` and `verify_task.py` exist,
    that is most of a wave.
  - `high` for planning and first-of-a-kind briefing: new wave plans, briefs
    for a new kind of work, judging reviews of core code.
  - `xhigh` for milestone gates and large re-plans.

  Set model and effort at session start (`/model`) and keep them. A change
  mid-session rebuilds the prompt cache. A Sonnet 5.5 manager for routine
  waves is a later trial, measured by `/usage` and outcomes against an Opus
  wave, once `oc.py` lands. The user approved this split on 2026-10-16.
- **On Pro (D28b) the manager is the largest Claude cost.** Start a fresh
  session per wave, keep wave sessions at `high`, send even small edits to
  the bookkeeper or an OpenCode worker, and never run two Claude workers at
  once when an OpenCode worker could take one of them. Opus 5.5, Sonnet 5.5
  and Haiku 5.5 all draw on the same five-hour and weekly limits.
- **What Claude's usage report showed (2026-10-16).** 88% of usage was at
  over 150k context, 27% came from subagent-heavy sessions, 14% from
  always-on context over 40k tokens, and 11% from the `/claude-api` skill.
  So: the context meter is wired before any other work (§8). Scouting goes
  to OpenCode `crpg-scout` on a Go model rather than `Explore`, which
  inherits the manager's Opus. Every Claude agent has a `tools:` allowlist
  (about 21k tokens saved per spawn). Unused MCP connectors are turned off
  for this project, and A0-8 trims the `AGENTS.md` files.
- **Effort per agent.** In the agent files: `architect` high, `implementer`
  high, `reviewer` high, `bookkeeper` low. A spawn may override.
- **Turn caps.** `maxTurns` on Claude workers; `--timeout` and the agent's
  `steps` on OpenCode workers.
- **Two attempts per rung, then escalate.** Never a third blind retry.
- **Output trimmed at the source.** Every tool prints a verdict line and at
  most 40 lines; `--full` and `--json` give more. Agents run `cargo test -q`.
- **Emergency brake.** `CLAUDE_CODE_SUBAGENT_MODEL=claude-sonnet-5-5` with
  `CLAUDE_CODE_SUBAGENT_MODEL_FORCE=1` in the settings `env` block puts every
  Claude subagent on Sonnet until removed.
- **Advisor trial (optional, D28-g).** The advisor tool works on
  subscriptions. Pairing Sonnet 5.5 implementers with an Opus 5.5 advisor may
  beat escalating them, but `advisorModel` also attaches to the Opus manager,
  so the trial must measure the manager's extra spend too.
- **Strip scaffolding as models improve.** Every harness piece encodes a
  guess about what a model cannot do. At each model change, re-run the
  bake-off and remove, one at a time, the pieces that no longer earn their
  cost.
- **Measure.** `oc.py` logs tokens and cost per run; `/usage` at each gate;
  `opencode stats --days 7` weekly. The `BACKLOG.md` throughput line gains
  "Claude usage per merged package" and "OpenCode runs per merged package",
  which it has never had.

## 10. Risks

| Risk | Early warning | Mitigation |
|---|---|---|
| Cheap worker passes visible tests and misses the contract | Hidden or reviewer-added tests fail | Contract-derived tests by a second family ("two keys"); review against the contract, not the tests |
| Free models disappear or change | `oc.py` exit 2 with `quota`, `402` or unknown model | Every tier lists two or more models from different labs; re-run the bake-off monthly (A0-7) |
| Go subscription limits run out mid-wave | Exit 2 `quota` | Per-model 5-hour and weekly limits are in `MODELS.md`; spread a wave across models |
| Manager overload | Long review queue, shallow reviews | Slot limit; scripts do the mechanics; the Claude reviewer for core |
| Context drift between sessions | The manager re-asks settled questions | Handover in files; fresh sessions; recall in every brief |
| Disk exhaustion | Build failures, `os error 112` | Slot count follows free space (§5.3); reuse slots |
| Prompt-cache churn | `/usage` cache-read share falls | Stable `AGENTS.md` (D28-f); pinned agent files |
| Data exposure on free endpoints | — | §4.2 rule 4; `external_directory` denied for every OpenCode agent |
| Scope creep | Diffs past the size budget | Owns list enforced by script; size budget in every brief |

## 11. Setup wave A0

A0 runs before Wave 4 continues (T024 is next). Rows marked **H** need your
decision first.

| ID | Package | Route | Done when |
|---|---|---|---|
| A0-1 | `.claude/agents/` (four files), `.claude/settings.json` permissions for the `tools/agents` scripts, `.agents/` and `BRIEF.md` in `.gitignore` | Manager | Each agent spawns on its pinned model; `Explore` still works |
| A0-2 | `opencode.json` and `.opencode/agents/crpg-{implement,tests,review,scout,grunt}.md` | Manager | `opencode run --agent crpg-scout` answers a question; a denied command is refused |
| A0-3 | `tools/agents/oc.py` (`run`, `wave`, `ask`, plus `record` and `stats` for the model record in `docs/agents/MODELS.md` §4.1) with tests in `tools/tests/` against a fake `opencode` | OpenCode models from an exact brief, head to head: Cat Jam Free and Gemini 4 Argon (rerouted from Claude 2026-10-16 by the user's preference) | Each failure mode in §5.1 has a test; a dry run prints the summary format |
| A0-4 | `tools/agents/verify_task.py` with planted-fault tests | OpenCode DeepSeek V4.1 Flash, the free endpoint (rerouted as A0-3) | Each §7.1 check fails on its planted fault and passes clean |
| A0-5 | `tools/agents/brief.py` and `recall.py` | OpenCode O1, following A0-3's style | `brief.py T023b` yields the contract sections only and is under a quarter of the file |
| A0-6 | Hooks: `SubagentStop` runs `verify_task.py` for Claude implementers; `PreToolUse` protects the files in §7.1; `SessionStart` (compact) re-injects the handover; A0-6b: `PostToolUse` context meter (§8) | Scripts by an OpenCode worker, wiring by the manager | Each hook blocks its planted case |
| A0-7 | Promote the 2026-10-16 bake-off harness (`docs/agents/bakeoff-2026-10-16/`) to `tools/agents/bakeoff/`, and add harder fixtures cut from this repository's history: a past task at its pre-implementation commit with its real tests hidden, and the code before a real fix (T019, T023d) as a review case | OpenCode O1 for the harness; the manager picks the fixtures | One command re-scores every configured model, and the new fixtures separate models the 2026-10-16 ones could not; about 20 fixtures, which also measure brief ordering and reasoning variant (`docs/agents/RESEARCH-2026-10-16.md`) |
| A0-8 | Context diet: the root `AGENTS.md` note moved to `PROJECT_STATE.md` on 2026-10-16 (D28f, done); next, trim crate `AGENTS.md` files, crate by crate | Manager drafts, you approve each crate | No information lost: each moved paragraph has a new home |
| A0-9 | Create slots 1 and 2 (disk freed 2026-10-16, D28-c) | Manager | `oc.py` finds both slots and each builds once |

## 12. Kickoff prompt for a manager session

You do not paste this: the root `CLAUDE.md` tells a session to follow it when
you say "start working".

```text
You are the manager of the CRPG engine build. You plan, brief, route, judge
and open PRs. You do not implement, you never read a worker's transcript, and
you never check by hand what a script checks. You never merge.

Read: docs/AGENT_TEAM_PLAN.md §§1–9, the handover at the top of
docs/PROJECT_STATE.md, and the in-flight rows of tasks/BACKLOG.md. Read
docs/agents/MODELS.md before routing and docs/agents/TEMPLATES.md before
briefing.

This session: the "Next" line of the handover, unless I name something else.

1. Confirm CI on master and resolve anything the handover lists as pending.
2. List the wave's ready packages with route, model and slot per §4.
3. Write each brief with tools/agents/brief.py plus your judgment.
4. Dispatch OpenCode packages with tools/agents/oc.py wave, Claude packages as
   background subagents. Stay within the slot limit.
5. Act on each summary: retry, escalate (§4.4), review (§7), or open the PR.
6. Have the bookkeeper record the results; check its diff.
7. Put non-blocking questions in the next decision-batch draft with a
   recommendation. Ask me directly only if the whole wave is blocked.
8. Never add a dependency, accept a golden, change a contract, touch the
   S-set without the gate, or switch your model mid-session.
9. Hand over at the end of the wave, or when the context meter says so
   (§8: finish the package at 150k, stop at 200k). Before stopping, update
   the handover in docs/PROJECT_STATE.md, naming any run still going.
```

## 13. Decisions for you (D28)

Answered 2026-10-16: a, c, d, e, f, g and h as recommended, b = Pro, plus D28i
(the `rustls` pin bump, S002). The record is `tasks/DECISIONS-2026-10-16.md`;
the table below is kept as asked.

| # | Decision | Recommendation |
|---|---|---|
| a | Adopt this plan as the plan of record for delegation | Adopt; run A0 next |
| b | Which Claude plan funds the Claude side | Tell the manager the plan; on Pro, route Claude implementers to Sonnet 5.5 only and keep Opus for A/C |
| c | Disk: delete the merged `crpg-t014` worktree (6 GB) and free more space for slots | **Done 2026-10-16 at your request:** worktree, merged branches, spike build folders and stale cache removed; 43 GB free, enough for two slots |
| d | Zen pay-as-you-go balance (currently $0; paid Zen models return 402) | Leave at $0. Go plus the free models cover the routing table; a top-up adds only GPT Sol/Astra, Gemini Pro and the like, which the Claude side already covers |
| e | Free models that keep or train on prompts may see this public repository's content | Allow, under §4.2 rule 4 |
| f | Move the root `AGENTS.md` "Note" status log to `docs/PROJECT_STATE.md`, leaving a one-line pointer | Move it; it costs every agent session and breaks prompt caching at every merge |
| g | Advisor trial on Claude implementers | Defer until A0 lands and one milestone has baseline numbers |
| h | `.github/workflows/opencode.yml` (untracked) runs `opencode/muse-spark-1.3`, a paid Zen model, and so fails with an empty balance; workflows are S-set | Do not commit it as is. If wanted, switch it to a free or Go model in a reviewed workflow task |

---

## Agent log

- 2026-10-16 (UTC) · claude-code + agent-team plan · Proposed a manager-and-workers delegation plan adapted from the two plans in `extras/`, routing implementation volume to OpenCode workers behind a verifying script and Claude subagents to A, C, S and core review, grounded in a same-day access test and bake-off; it is pending the user's D28 answers and changes no gate, contract or task order.
- 2026-10-16 (UTC) · claude-code + disk cleanup · Recorded the same-day cleanup the user asked for (merged worktree, merged branches, stale build caches) and moved §5.3 from one build slot to two, since 43 GB is now free.
- 2026-10-16 (UTC) · claude-code + D28 adopted · Marked the plan adopted with the user's D28 answers and applied D28b (Pro): Claude workers stop at Sonnet 5.5, Opus 5.5 is kept to A and C packages, escalations past Sonnet go to the manager, and §9 adds the Pro cost rules.
- 2026-10-16 (UTC) · claude-code + user model preference · Applied the user's free-then-Go-then-Claude order to §4.3's default routes and A0-3/A0-4, and added `record`/`stats` to A0-3 for the model record, so that Claude is spent on design and judgment rather than exact-spec work.
- 2026-10-16 (UTC) · claude-code + user correction on model choice · Reworded the preference as the strongest suitable OpenCode model, then Claude, with free versus Go only a tiebreak. Restored O1 as the default worker tier in §4.3 and recorded the A0-3 head-to-head.
- 2026-10-16 (UTC) · claude-code + user preference, third statement · Reworded the preference note: intelligent free models first, then cheap and then expensive Go, with Claude at the manager's discretion, as the user asked once the Go cap was hit.
- 2026-10-16 (UTC) · claude-code + user question on manager effort and handover · Set the manager's effort by session type (medium, high, xhigh), added handover thresholds at 150k and 200k tokens enforced by a context-meter hook, and recorded why managers relay rather than nest. The first manager session's context reached about 346k tokens, which is what prompted this.
- 2026-10-16 (UTC) · claude-code + user request · Made §12 the prompt that the root `CLAUDE.md` triggers on "start working", with the session's work taken from the handover, so the user no longer pastes a kickoff prompt.
- 2026-10-16 (UTC) · claude-code + research fold-in · Took three rules from the citation-checked R5–R7 reports: `oc.py wave` keeps one package per crate, a known-flake failure is an infrastructure exit, and A0-7 aims at about 20 fixtures that also measure brief ordering. The rest is recorded in `docs/agents/RESEARCH-2026-10-16.md`.
- 2026-10-16 (UTC) · claude-code + usage report · Added the user-supplied Claude usage breakdown to §9 with the rule each figure drives, so later sessions cost less.
