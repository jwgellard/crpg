# Model catalog and routing table

Companion to `docs/AGENT_TEAM_PLAN.md`; the manager reads it before routing or
escalating. It is the only file that changes when models, prices or access
change. **Checked 2026-10-16 (UTC)** on this machine with OpenCode 1.18.35 and
Claude Code 2.1.301. Re-check monthly (A0-7), and whenever `oc.py` exits 2
with `quota`, `402` or an unknown model.

Free models on Zen are "free for a limited time"; stealth models (Big Pickle,
Space Bunny, Cat Jam) do not name their lab. Treat every row as perishable.

---

## 1. Access on this machine

Tested 2026-10-16 with a one-word prompt per model, then the bake-off (§5).

| Route | State | Detail |
|---|---|---|
| OpenCode Zen, free models | **Works** | 10 of 11 free models answered. `nemotron-3-ultra-free` timed out twice (overloaded) |
| OpenCode Zen, paid models | **Blocked: 402 "Insufficient account funds"** | Balance is $0. Topping up is D28-d |
| OpenCode Go subscription | **Works** | All 17 Go models tried answered. Limits per model in §3.2 |
| OpenAI through ChatGPT sign-in | **Partial** | `gpt-6-luna`, `gpt-5.6-luna` and `gpt-5.6-terra` work. `gpt-6-sol`, `gpt-6.1-sol`, `gpt-6-astra`, `gpt-5.6-sol`, `gpt-5.5` and `gpt-5.4` are refused: "not supported when using Codex with a ChatGPT account". Earlier sessions used Astra and Sol, so the ChatGPT plan has changed |
| OpenRouter `:free` models | **Works** | `cohere/north-mini-code`, `thinkingmachines/inkling` and `poolside/laguna-s-2.1` answered; `nvidia/nemotron-3-ultra` timed out. About 50 requests a day without credit |
| DeepSeek direct API | **Blocked: "Insufficient Balance"** | — |

`opencode stats` (all time, every project): 236 sessions, 11,535 messages,
1.95 B tokens of which 1.88 B were cache reads, **$1.34 total**. The volume
came from `muse-spark-1.3-contributor-free` (7,458 messages), `big-pickle`
(1,002), Go `muse-spark-1.3-contributor` (817) and ChatGPT-plan GPT models.

### Running OpenCode from a script

Found while testing; `tools/agents/oc.py` must handle each one.

1. `opencode run` waits for ever on an open stdin. Close it (`< /dev/null`,
   `stdin=DEVNULL`).
2. OpenCode takes its working directory from the inherited `PWD`, not the
   process cwd. Pass `--dir <slot>` **and** set `PWD=<slot>`. With only the
   cwd set, workers read the parent directory.
3. A worker that can see a solution will use it. In the first, mis-rooted
   bake-off run a model found the reference implementation and hidden tests in
   a sibling directory and copied both into the task template. Deny
   `external_directory` and keep references out of the slot.
4. Concurrent starts race on OpenCode's SQLite database: some runs failed at
   once with `database is locked` at four to six simultaneous starts. Start
   runs at least 4 s apart and retry on that message.
5. Permissions set to `ask` are auto-rejected in `opencode run`; write every
   rule as `allow` or `deny`. Never pass `--auto`.
6. On Windows, `opencode` resolves to the npm shim `opencode.cmd`, so the
   prompt passes through `cmd.exe`: a `<`, `>`, `|` or `&` in it is parsed as
   shell syntax. The review prompt's `<line number>` failed every review run
   with "The system cannot find the file specified." Call
   `%APPDATA%\npm\node_modules\opencode-ai\bin\opencode.exe` directly, and
   put long instructions in `BRIEF.md` rather than on the command line.
7. Every run carries roughly 9–13k tokens of OpenCode system prompt and tool
   definitions before the brief.
8. After a fatal provider error (`Go usage limit exceeded`, or `Too Many
   Requests` after three retries), `opencode run` does not exit and prints
   nothing. The error is only in OpenCode's own log,
   `~/.local/share/opencode/log/opencode.log`. A free model also went silent
   for 43 minutes after a completed tool call with no error logged. `oc.py`
   needs an idle watchdog that reads that log (A0-3).
9. Session titles go to a "small model", which defaulted to the paid Zen
   `gpt-5.4-nano` and failed with 402 on every run. `opencode.json` sets
   `small_model` to a free model.
10. **The free models share one rate limit across the account.** On
   2026-10-16, with 7–8 OpenCode runs going at once, every free model began
   refusing with `Rate limit exceeded`: DeepSeek V4.1 Flash, Muse Spark, GLM-5.3
   Flash, Big Pickle and Space Bunny. Four runs failed before their first
   step. Run **at most three free runs at once in total**, and at most one per
   model. `oc.py wave` must enforce both caps. **Once hit, the limit takes
   about 12 hours to reset** (the user, 2026-10-16). It was hit again at about
   14:00 UTC that day: V8 on GLM-5.3-Flash Free and V10 on Muse Spark Free
   both failed at step 0. After a hit, route every run to its Go ID until the
   next day rather than retrying free models.
11. **Two free models are exempt in practice.** OpenCode's Go page (read
   2026-10-16 12:10 UTC) lists LongCat 2.5 Preview Free and Cat Jam Free as
   **Unlimited** for Go subscribers, both "limited time" and Cat Jam
   "limited regions". At 12:11 UTC both answered while DeepSeek V4.1 Flash
   Free was still refused, and that morning they had kept working through
   the shared limit. Item 10's caps apply to the other free models.
   **Use the Go route, not the free route.** Both models have two IDs:
   `opencode/cat-jam-free` and `opencode/longcat-2.5-preview-free` go
   through the free Zen tier and share its account-wide rate limit (item
   10); `opencode-go/cat-jam-free` and `opencode-go/longcat-2.5-preview-free`
   go through the Go subscription, which lists them as unlimited. On
   2026-10-16 the free IDs refused every run from 12:19 UTC (LongCat) and
   12:25 UTC (Cat Jam) and were still refusing at 12:41, while the Go IDs
   answered at once. The user spotted the difference. Always route these two
   by their `opencode-go/` IDs.
12. **OpenCode 2.x (since 2026-10-16 about 14:28 UTC).** OpenCode 1.18.35
   (`opencode-ai`) was replaced by 2.0.31 (`@opencode/cli`); the executable is
   now `%APPDATA%\npm\node_modules\@opencode\cli\bin\opencode.exe`. The
   replacement killed two runs in flight (`database is locked` mid-run, not
   only at start). `opencode run` in 2.x has no `--dir` and no `--variant`:
   the workspace is the process cwd plus `PWD`, and a variant is part of the
   model (`-m provider/model#variant`). Runs share a background service unless
   `--standalone` is given, so every scripted run passes it. The JSON events
   are unchanged; a provider failure is a new top-level event
   `{"type":"error","error":{"type":"provider.no-route",...}}`. Cat Jam is no
   longer offered under either route. `oc.py` (A0-3) detects the version with
   `--version` and builds the right command for 1.x or 2.x. A permission
   pattern denies a whole compound command when any part matches (for example
   `git branch --show-current` inside a `;` chain), and one worker read that
   single refusal as "shell denied" and stopped: briefs now name the denied
13. **Beam Free is Cat Jam renamed (the user, 2026-10-16: "a BeamFree model
   that is unlimited on Go"; "Beam is just Cat Jam as a new name, so it
   should take over Cat Jams tasks").** Every rule, tier place and
   field-record row written for Cat Jam in this file applies to
   `opencode-go/beam-free`, and the uncapped Go models are Beam and
   `opencode-go/longcat-2.5-preview-free`; both keep working after the Go
   5-hour cap. Never route `opencode/beam-free`, which draws on the free Zen
   pool; `oc.py` refuses it and both Cat Jam IDs, naming the Go Beam ID.
   First runs under the new name: the T108 blind ADR draft, 395 lines in
   88 s at $0.00 (unreviewed); then A0-8 (crpg-cli `AGENTS.md` trim).
   **Many free reviews, collated (the user, 2026-10-16):** "Feel free to run
   multiple reviews through the unlimited models since they are completely
   free. Happy to have multiple review sessions running with them that
   collate into an overall finding." Run several independent reviews of one
   package on Beam and LongCat (within the three-runs-per-model cap) and
   merge their findings into one verdict before any Kimi K3 or Gemini 4 Argon
   review. A finding counts once however many reviewers raise it; a finding
   only one reviewer raises still gets the manager's check.

## 2. Claude models (subagents in Claude Code)

Prices are Anthropic API rates per million tokens; on a subscription they
indicate relative draw on the plan's limits. All four have a 1M context
window and 128K maximum output. Thinking is always on; effort is the lever.

| Model | ID | Input / output / cache read | Default effort | Role in this plan |
|---|---|---|---|---|
| Fable 5.1 | `claude-fable-5-1` | $10 / $50 / $0.25 | high | Only on your request: a design you have lost confidence in. Bills to usage credits on some plans |
| Opus 5.5 | `claude-opus-5-5` | $4 / $20 / $0.20 | **medium** — set it | Manager; `architect` for A and C packages only. On Pro (D28b) it is never an implementer or a review rung |
| Sonnet 5.5 | `claude-sonnet-5-5` | $2 / $10 / $0.20 | high | Default Claude subagent: `implementer`, `reviewer`, pattern-following contracts; the top implementer rung on Pro |
| Haiku 5.5 | `claude-haiku-5-5` | $0.10 / $0.50 / $0.01 up to 100K prompt; $0.50 / $2.50 above | **medium** — set it | `bookkeeper`; mechanical Claude-side edits |

Notes for routing:

- One aggregator's weighted coding score (BenchLM, 2026-10-15) ranks Sonnet
  5.5 at 84.1, Opus 5.5 at 82.6, Fable 5.1 at 76.8 and Haiku 5.5 at 53.0. It
  is a single, unaudited composite, but it supports Sonnet 5.5 as the default
  Claude implementer, with Opus 5.5 kept for design judgment.
- Haiku 5.5 costs a fortieth of Opus 5.5 per input token while the prompt
  stays under 100K; past that its price rises fivefold, so give it small,
  self-contained jobs.
- Haiku 5.5 is also on OpenCode Go (`opencode-go/claude-haiku-5-5`), which
  draws on the Go limits instead of the Claude plan.
- The advisor tool accepts, for a Sonnet 5.5 main model, Fable or Opus 5 and
  later, or Sonnet 5.5; for an Opus 5.5 main model only Fable or Opus 5 and
  later (D28-g).

## 3. OpenCode models

### 3.1 Tiers

| Tier | Meaning | Default use |
|---|---|---|
| **O1** | Strong: passed all three bake-off tasks cleanly and has a usable request budget | Implementation from a contract, tests from a contract, first-pass review |
| **O2** | Workhorse: correct on the scout and review tasks, cheap or plentiful | Bookkeeping, scouting, content loops, second opinions, simple implementation |
| **O3** | Free fallback | Scouting and bookkeeping when Go limits run out; never S-set or core |

### 3.2 Limits and privacy

Go limits are dollar amounts per model per month, with 20% available per
5-hour window and 50% per week; the request counts are OpenCode's own
estimates for the $10 Go tier. **The cap that binds is shared across the account.** On
2026-10-16 Go refused every model at 10:17 UTC, Muse Spark on its first
request, after $5.01 of notional cost that day: about 400 requests, most of
them the 07:00 UTC bake-off. Kimi K3 ($1.26), Gemini 4 Argon ($1.22), GLM-5.3
($0.67), Qwen3.8 Max ($0.62) and Step 5 ($0.47) made up four fifths of it.
Price per request varies about 400-fold, from Muse Spark Contributor at
45,300 requests per 5 hours to Kimi K3 at 110. A reasoning stall on an
expensive model costs the most: Kimi K3's single 32k-token thinking step cost
$0.51 and produced nothing. A bake-off implementation run took 5–42 model
requests (median 10), so divide the 5-hour figure by 10 to 20 for
"implementation runs per window". Privacy is from OpenCode's Zen and Go pages on 2026-10-16.

| Model ID | Lab | Go requests per 5 h / month | Retention and training |
|---|---|---|---|
| `opencode-go/kimi-k3` | Moonshot | 110 / 490 | 0-day, no training |
| `opencode-go/glm-5.3` | Z.ai | 220 / 1,080 | 0-day, no training |
| `opencode-go/qwen3.8-max` | Alibaba | 160 / 810 | 0-day, no training |
| `opencode-go/deepseek-v4-pro` | DeepSeek | 1,050 / 5,200 | 0-day under a ZDR agreement to 2026-10-31 |
| `opencode-go/mimo-v2.6-pro` | Xiaomi | 3,250 / 16,300 | 0-day, no training |
| `opencode-go/step-5-preview` | StepFun | 1,720 / 8,520 (promotional) | 0-day, no training |
| `opencode-go/hy4-preview` | Tencent | 1,350 / 6,770 | 0-day, no training |
| `opencode-go/gemini-4-argon-contributor` | Google | 440 / 2,120 (promotional) | **Training use** |
| `opencode-go/kimi-k2.7-code` | Moonshot | 1,350 / 6,750 | 0-day, no training |
| `opencode-go/minimax-m3` | MiniMax | 3,200 / 16,000 | 0-day, no training |
| `opencode-go/deepseek-v4.1-flash` | DeepSeek | 34,670 / 173,330 | 0-day under a ZDR agreement to 2026-10-31 |
| `opencode-go/qwen3.8-flash` | Alibaba | 5,400 / 27,000 | 0-day, no training |
| `opencode-go/glm-5.3-flash` | Z.ai | 6,320 / 31,580 | 0-day, no training |
| `opencode-go/gpt-6-luna` | OpenAI | 4,230 / 21,130 | 30-day, no training |
| `opencode-go/claude-haiku-5-5` | Anthropic | 3,850 / 19,230 | 30-day, no training |
| `opencode-go/muse-spark-1.3-contributor` | Meta | 45,300 / 226,600 | **Training use** (the discount's price) |
| `opencode/muse-spark-1.3-contributor-free` | Meta | free | **Training use** |
| `opencode/big-pickle` | undisclosed | free | Data may be used to improve the model |
| `opencode/mimo-v2.6-flash-free` | Xiaomi | free | Data may be used to improve the model |
| `opencode/space-bunny-free` | undisclosed | free (also on Go) | 0-day, no training |
| `opencode/cat-jam-free` | undisclosed | free | 30-day, no training; region-restricted |
| `opencode/longcat-2.5-preview-free` | Meituan | free | 0-day, no training |
| `opencode/deepseek-v4.1-flash-free` | DeepSeek | free | 0-day, no training |
| `opencode/glm-5.3-flash-free` | Z.ai | free | 0-day, no training |
| `opencode/nemotron-3.5-*-free` | NVIDIA | free | **Trial only; logged** — scouting only |
| `openai/gpt-5.6-terra` | OpenAI | ChatGPT plan | OpenAI account terms |

### 3.3 Bake-off results (2026-10-16)

Three small tasks, one attempt each, default reasoning variant, run through
OpenCode in isolated sandboxes; method and caveats in §5. "Cost" is
OpenCode's notional price, which counts against the Go limits.

| Model ID | Scout | Implement (hidden tests, gates) | Review (planted bugs found, false positives) | Implement: requests, time, tokens, cost |
|---|---|---|---|---|
| `opencode-go/deepseek-v4.1-flash` | exact | 21/21, all gates | 5/5, 0 | 8 · 60 s · 125k · $0.007 |
| `opencode-go/kimi-k3` | exact | 21/21, all gates | 5/5, 0 | 10 · 119 s · 158k · $0.25 |
| `opencode-go/mimo-v2.6-pro` | exact | 21/21, all gates | 5/5, 0 | 10 · 398 s · 260k · $0.03 |
| `opencode-go/gemini-4-argon-contributor` | exact | 21/21, all gates | 5/5, 0 | 8 · 297 s · 210k · $0.59 |
| `opencode-go/deepseek-v4-pro` | exact | 21/21, all gates | 5/5, 0 | 6 · 229 s · 107k · $0.04 |
| `opencode-go/claude-haiku-5-5` | exact | 21/21, all gates | 5/5, 0 | 5 · 40 s · 82k · $0.007 |
| `opencode-go/gpt-6-luna` | exact | 21/21, all gates | 5/5, 0 | 13 · 88 s · 165k · $0.007 |
| `opencode-go/glm-5.3-flash` | exact | 21/21, all gates | 5/5, 0 | 10 · 161 s · 159k · $0.011 |
| `opencode-go/qwen3.8-flash` | exact | 21/21, all gates | 5/5, 0 | 19 · 453 s · 531k · $0.03 |
| `opencode-go/qwen3.8-max` | exact | 21/21, all gates | 5/5, 0 | 14 · 728 s · 415k · $0.40 |
| `opencode-go/kimi-k2.7-code` | exact | 21/21, all gates | 5/5, 0 | 8 · 430 s · 171k · $0.11 |
| `opencode-go/hy4-preview` | exact | 21/21, all gates | 5/5, 0 | 24 · 577 s · 590k · $0.10 |
| `opencode-go/glm-5.3` | exact | **stalled**: no code, finish `length` | 5/5, 0 | 4 · 502 s · 48k · $0.18 |
| `opencode-go/minimax-m3` | exact | **stalled**: no code, finish `length` | 4/5, 0 (missed the cast) | 4 · 325 s · 48k · $0.05 |
| `opencode-go/step-5-preview` | exact | 21/21 but **timed out** at 900 s looping on fmt and clippy | 5/5, 0 | 35 · 900 s · 1.7M · $0.37 |
| `opencode/cat-jam-free` | exact | 21/21, all gates | 5/5, 0 | 11 · **34 s** · 173k · free |
| `opencode/deepseek-v4.1-flash-free` | exact | 21/21, all gates | 5/5, 0 | 8 · 113 s · 141k · free |
| `opencode/glm-5.3-flash-free` | exact | 21/21, all gates | 5/5, 0 | 5 · 143 s · 74k · free |
| `opencode/longcat-2.5-preview-free` | exact | 21/21, all gates | 5/5, 0 | 8 · 209 s · 129k · free |
| `opencode/muse-spark-1.3-contributor-free` | exact | 21/21, all gates | 5/5, 0 | 17 · 127 s · 368k · free |
| `opencode/big-pickle` | exact | 21/21, all gates | 5/5, 0 | 8 · 342 s · 148k · free |
| `opencode/space-bunny-free` | exact | 21/21, all gates | 5/5, 0 | 42 · 498 s · 1.5M · free |
| `opencode/nemotron-3.5-super-vl-free` | exact | 21/21, all gates | 4/5, 0 (missed the cast) | 12 · 208 s · 354k · free |
| `opencode/mimo-v2.6-flash-free` | exact | **stalled**: no code, finish `length` | 5/5, 0 | 4 · 565 s · 49k · free |
| `opencode/nemotron-3.5-lightning-free` | timed out | timed out | timed out | — |
| `openai/gpt-5.6-terra` (ChatGPT plan) | **wrong**: missed the four "any crate" rows | 21/21, all gates | timed out | 12 · 174 s · 166k · plan |

What the results say:

- **Access and rule-following are not the problem.** Every model that
  produced code kept to the rules: no floats, no edited tests, no new
  dependencies, no `unsafe`.
- **"Stalled" is one failure mode, not a weak model.** GLM-5.3, MiniMax M3 and
  MiMo-V2.6-Flash each spent the whole 32,000-token output allowance reasoning
  in one step, wrote no code and ended with finish reason `length`. `oc.py`
  treats `length` with no edits as an infrastructure exit (`no-edit`) and
  retries once with `--variant low` and the instruction to write the code in
  small edits. GLM-5.3 stays a reviewer until it passes a retry.
- **The tasks were too easy to rank the strong models.** An exact contract let
  free models match paid ones. The ranking for hard work below therefore also
  leans on public composites, which are weaker evidence, until A0-7 adds
  harder fixtures from this repository.
- **Speed and request budget separate models more than correctness did.**
  DeepSeek V4.1 Flash and Cat Jam finished in about a minute; Qwen3.8 Max,
  Hy4 and Step 5 took 8–15 minutes for the same result.

### 3.4 Public composite scores

BenchLM's weighted coding composite (verified 2026-10-15) for the models
above: Gemini 4 Argon 72.3, MiMo-V2.6-Pro 63.3, GPT-5.6 Terra 61.9, DeepSeek
V4.1 Flash 60.7, Step 5 Preview 60.5, MiMo-V2.6-Flash 60.1, Kimi K3 58.9,
Hy4 preview 56.7, GLM-5.3 56.4, Qwen3.8 Max 53.6, GPT-6 Luna 53.2, Claude
Haiku 5.5 53.0, GLM-5.3-Flash 48.3, DeepSeek V4 Pro 47.2, Kimi K2.7 Code
41.8, MiniMax M3 36.4, Nemotron 3 Ultra 21.8, Nemotron 3.5 Lightning 16.0.
One aggregator, unaudited, with labs' own harnesses behind many inputs: use it
to break ties, never to overrule a bake-off failure.

## 4. Routing table

**Preference (the user, 2026-10-16): stretch tokens.** Among the models
strong enough for the job, use up the intelligent free models' limits first:
Muse Spark 1.3 Free, DeepSeek V4.1 Flash Free, GLM-5.3-Flash Free, LongCat
Free. Then use cheap Go models, and keep expensive Go models (Kimi K3,
Gemini 4 Argon) for reviews and escalations. Never take a weaker model only
because it is free. Claude tokens are
kept for higher-level thinking: A and C packages, the S-set, which
`docs/AGENT_TEAM_PLAN.md` §4.2 keeps Claude-only, judgment review, and
escalations an O1 model failed. "Strongest" means the tier below and the §4.1
field record. Composite scores only break ties.

**Two free lanes, kept apart (the user, 2026-10-16):** "Make sure that, in
the future, the free Zen tokens go to models like Muse Spark and other good
models that have a free Zen version." The free Zen tier is one account-wide
rate limit (§1 item 10), so spend it only on strong models with a free Zen
variant: `opencode/muse-spark-1.3-contributor-free`,
`opencode/deepseek-v4.1-flash-free` and `opencode/glm-5.3-flash-free`, at
most three runs at once. LongCat and Cat Jam never use the Zen route; they
run on their unlimited `opencode-go/` IDs (§1 item 11). `oc.py` should
refuse an `opencode/cat-jam-free` or `opencode/longcat-2.5-preview-free`
model ID and name the Go ID instead.

**LongCat and Cat Jam first, once proven (the user, 2026-10-16):** "Long cat
and cat jam are free right now. If they can be assessed to be reliable, I
would like to utilise them heavily." Both scored 21/21 and 5/5 in the
bake-off, but that fixture was too easy to rank strong models (§3.3). Their
assessment is the §4.1 field record, run on real work from 2026-10-16:
research reports citation-checked by the other model, and implementation
(A0-6 on LongCat, T097-m on Cat Jam) through verify and review. Promote
either to first choice in a role once it has 5 runs there and a
first-attempt pass rate at least the incumbent's. Against Cat Jam so far:
one model-caused 43-minute hang (A0-3); a 10-minute idle watchdog now bounds
that cost. Higher-allowance Go models come next for implementation: Muse
Spark 1.3 Contributor (45,300 requests per 5 h), DeepSeek V4.1 Flash
(34,670), MiMo-V2.6-Flash and MiMo-V2.5 (30,100 each).

Not yet benchmarked, from the same Go page: LongCat-2.0 (11,400 per 5 h),
MiMo-V2.5 and V2.5-Pro, Qwen3.7 Plus, GLM-5.2, Hy3, Grok 4.7, GPT 5.6 Luna
and Muse Spark 1.2 Contributor on Go; Exo, MiMo-V2.5, LL Alpha and Jev 1.13
on the free list. They wait for A0-7's fixtures.

The principle: **plentiful models implement, scarce strong models review.**
On Go that also means **cheap**: implement on Muse Spark Contributor,
DeepSeek V4.1 Flash or MiMo-V2.6-Pro, which cost fractions of a cent per
request. Keep Kimi K3 and Gemini 4 Argon for reviews (2–5 requests) and
escalations, and never run them head to head on a long implementation or
measurement run. The manager learned this on 2026-10-16 by doing it.
A bake-off implementation run took 5–42 requests (median 10); a review took
2–5. Kimi K3's 110 requests per 5 hours is about ten implementations but
thirty reviews.

| Work | First choice | Second, another lab | When Go limits run out |
|---|---|---|---|
| I outside the S-set, routine | `opencode-go/deepseek-v4.1-flash` | `opencode-go/muse-spark-1.3-contributor` or `opencode-go/mimo-v2.6-pro` | `opencode/deepseek-v4.1-flash-free` |
| I outside the S-set, harder or after one failure | `opencode-go/kimi-k3` | `opencode-go/gemini-4-argon-contributor` | Claude `implementer` |
| Tests from a contract ("two keys") | A lab other than the implementer's: `opencode-go/mimo-v2.6-pro` or `opencode-go/kimi-k3` | `opencode-go/gemini-4-argon-contributor` | `opencode-go/deepseek-v4.1-flash` |
| First-pass review | `opencode-go/gemini-4-argon-contributor` | `opencode-go/kimi-k3`, or `opencode-go/glm-5.3` | `opencode-go/deepseek-v4-pro` |
| Scouting | `opencode-go/deepseek-v4.1-flash` | `opencode-go/muse-spark-1.3-contributor` | Built-in `Explore` |
| Bookkeeping, commit and PR text | `opencode-go/muse-spark-1.3-contributor` | `opencode-go/gpt-6-luna` | Claude `bookkeeper` |
| G: content through `crpgc validate --json` | `opencode-go/deepseek-v4.1-flash` | `opencode-go/glm-5.3-flash` | `opencode/glm-5.3-flash-free` |
| S-set, A, C, core review | Claude only (`docs/AGENT_TEAM_PLAN.md` §§4.2–4.3) | — | — |

Tier membership:

- **O1:** DeepSeek V4.1 Flash (Go), Kimi K3, MiMo-V2.6-Pro, Gemini 4 Argon
  Contributor, DeepSeek V4 Pro, and Muse Spark 1.3 Contributor (Go, and the
  free variant). Five labs, so a second opinion can always come from another
  family. Muse Spark was promoted from O3 on 2026-10-16 at the user's prompt.
  Its O3 place came from its training-use terms, not its results: it
  scored 21/21 and 5/5 in the bake-off, its signature is on 92 Markdown files in
  this repository since 2026-09-05 (the E009–E022 spec-gap triage, the
  ADR-0008/0009 decisions, the `f64` ban, status catch-ups through T019), it is
  the most-used model here by volume, and its Go variant has the largest request
  budget of any model.
- **O2:** GPT-6 Luna, Claude Haiku 5.5 (Go), GLM-5.3-Flash, Qwen3.8 Flash,
  Kimi K2.7 Code, GLM-5.3 (review only until it passes an implementation
  retry), and Beam Free, formerly Cat Jam Free (fast, but a stealth model
  whose terms can change; §1 item 13).
- **O3:** DeepSeek V4.1 Flash Free, GLM-5.3-Flash Free, LongCat 2.5 Preview
  Free, Big Pickle, Space Bunny Free (correct
  but slow and token-hungry), Qwen3.8 Max and Hy4 preview (correct but slow;
  Qwen3.8 Max is also scarce). O3 is a fallback when O1 and O2 are unavailable,
  not a first choice.
- **Not routed:** Nemotron 3.5 Lightning and Nemotron 3 Ultra (timeouts),
  Nemotron 3.5 Super VL and MiMo-V2.6-Flash Free (scouting at most), MiniMax
  M3 (stalled and missed a bug), Step 5 Preview for implementation (gate
  loop), and GPT-5.6 Terra for scouting or review.

Data handling: Gemini 4 Argon Contributor, the Muse Spark contributor models,
Big Pickle and MiMo-V2.6-Flash Free may train on prompts. That is acceptable
for this public repository under `docs/AGENT_TEAM_PLAN.md` §4.2 rule 4 and
D28-e; if D28-e is refused, drop them and promote Kimi K3 to first-pass
reviewer.

### 4.1 Field record: demote and promote

The bake-off ranks models once; real tasks rank them continuously. On
2026-10-16 the user asked that models which keep underperforming be avoided,
narrowing routing to the best.

- Every `oc.py` run leaves a result record, and the manager adds review and
  merge outcomes with `oc.py record`, Claude subagent runs included.
  `oc.py stats` ranks each model per role by first-attempt pass rate: the run
  passed `verify_task.py` and review did not send it back.
- **Demote** a model from a role when it fails 2 of its last 4 runs in that
  role. A failure is a verify failure, a review reject, or an infra exit the
  model caused (`no-edit`, `timeout`). Quota, `402` and `locked` exits are
  not the model's fault and do not count. A first choice drops to second, and
  a second choice drops back to its tier list. A model demoted twice in a
  role leaves that role's routing.
- **Promote** a model to first choice for a role once it has at least 5 runs
  in that role and the best first-attempt pass rate among models with 5 or
  more. Ties go to the cheaper, then the faster, model.
- The manager runs `oc.py stats` before routing a wave and applies the rule
  at each gate. Every change is recorded here with its date and numbers. A
  change that moves work between Claude and OpenCode is still a plan change
  (§6).
- **Head-to-head runs build the evidence.** When the same brief can go to
  two O1 models from different labs without a build-slot conflict, run both.
  Both results enter the record, and the better one (verify first, then
  review) goes to the PR. The first was A0-3, Cat Jam Free against Gemini 4
  Argon, on 2026-10-16. A0-7's harder fixtures, run several times per model,
  are the systematic version.
- Claude subagents keep the same record. On Pro (D28b), when Sonnet 5.5 keeps
  failing one kind of package, that kind goes back to the manager to split,
  not up to Opus.

**Record, 2026-10-16 (hand-run through `ocrun.py`, before `oc.py record`).**
Infra exits (rate limit, quota) are listed but do not count.

| Model | Role | Runs and outcome |
|---|---|---|
| `opencode-go/deepseek-v4.1-flash` | implement | A0-4 attempt 2 and A0-6 attempt 2: both verified and reviewed "ready for PR" first time ($0.026, $0.012) |
| `opencode-go/deepseek-v4.1-flash` | research | R4 and R10-deep written; **V8 failed**: announced the results file and stopped without writing it (counts) |
| `opencode-go/muse-spark-1.3-contributor` | implement | A0-3: verified (0/9 checks failed), 32 tests, about twice the size budget; live smoke test still due ($0.057) |
| `opencode-go/muse-spark-1.3-contributor` | research | V3 and V4 citation checks complete; R14-deep written (30+ sources, $0.071) |
| `opencode-go/gemini-4-argon-contributor` | review | A0-4 and A0-6 reviews: precise, item by item, $0.39 each |
| `opencode/cat-jam-free` → use `opencode-go/cat-jam-free` | research | Six first-pass reports: **68 of 70 checkable citations held** (R9 11/11, R11 20/20, R13 11/11, R15 12/12, R8 6/7, R12 8/9). Deep passes R8, R12, R13 written, R9-deep on the Go route; checks pending. Fast: 30–80 s a report |
| `opencode/cat-jam-free` | implement | A0-3 earlier: 43-minute hang (counts). T097-m: rate limit (infra) |
| `opencode/longcat-2.5-preview-free` → use the Go ID | research, implement | R10 first pass written; A0-6, R14, R4, V3–V6 lost to the free-tier rate limit (infra) |
| `opencode-go/muse-spark-1.3-contributor` | implement | A0-3 attempts 3–4 (second session): attempt 3 fixed all 15 notes with tests, but Kimi K3's second review found 4 more (counts as one `review-fix`); attempt 4 stopped after one refused shell call it misread as "shell denied" (counts), then finished on a rerun |
| `opencode-go/deepseek-v4.1-flash` | implement | A0-5: verified first time; Gemini review "fix" (3 high), then "ready for PR" after one round. A0-6b attempt 2: "ready for PR". A0-3 attempt 5 (escalated): all four findings fixed with tests, accepted by the manager |
| `opencode-go/kimi-k3` | review | A0-3 twice: found two real high defects (a Windows liveness probe that kills, infra classification of file contents), then four narrow ones; precise file:line findings |
| `opencode-go/gemini-4-argon-contributor` | review | A0-6b attempt 2 "ready for PR"; A0-5 "fix" with three real high findings in 19 task files, then "ready for PR" |
| `opencode-go/mimo-v2.6-pro` | research | V8 complete (81 rows over R8/R12/R13-deep) although the process ended rc=1 mid-run (infra) |
| `opencode/glm-5.3-flash-free`, `opencode/muse-spark-1.3-contributor-free` | research | V8 and V10: rate limit at step 0, about 14:00 UTC (infra) |

## 5. Bake-off method

Run 2026-10-16. The harness, fixtures and scored results are in
`docs/agents/bakeoff-2026-10-16/`; A0-7 promotes the harness to
`tools/agents/bakeoff/`.

| Task | What the worker got | Scored by |
|---|---|---|
| Scout | A copy of `AGENTS.md` and `tools/lint/deps.py`; "which crates may depend directly on `crpg-sim`? Reply with a JSON array" | Exact match with the eight crates the `ALLOWED` table permits, including its four "any crate except" rows |
| Implement | A one-file Rust crate with three `todo!()` functions (16.16 fixed-point multiply with ties away from zero, truncating divide with saturation, and an 18-digit decimal parser with exact rounding), an `AGENTS.md` with this repository's style of rules, and four visible tests | 21 hidden edge-case tests; `cargo fmt --check`, `cargo clippy -D warnings` and the visible tests; and the rules: no floats, no `unsafe`, no edits to `Cargo.toml` or the visible tests |
| Review | A 44-line combat module and a five-rule spec, with five planted defects (missing tie-break, `< 0` for "zero or below", unchecked overflow, `HashMap` iteration order, an `as i32` cast that can lower hp) and one decoy (FNV-1a's intended wrapping multiply) | Defects found by line and keyword; false positives; whether the decoy was flagged |

Limits: one attempt per model per task, default reasoning variant, small
self-contained tasks rather than work in this 16-crate workspace, and OpenCode
as the only harness. Three harness faults were found and fixed during the run
(§1, items 2, 3 and 6); only clean reruns are reported.

## 6. Recalibration

Monthly, after any `oc.py` exit 2 for `quota`, `402` or an unknown model, and
whenever Zen or Go changes its model list:

1. `opencode models opencode`, `opencode models opencode-go` and
   `opencode models openai`; ping every routed model.
2. Run the bake-off (A0-7) on the routed models plus any new ones.
3. Update §§1, 3 and 4 here, with the date, and nothing else. A routing change
   that moves work between Claude and OpenCode is a plan change and goes to
   the next gate.

---

## Agent log

- 2026-10-16 (UTC) · claude-code + agent-team plan · Recorded the same-day access test and three-task bake-off of 26 OpenCode models with the Claude lineup, limits and privacy terms, and derived the routing table and tiers from them, noting where the tasks were too easy to rank and public composites fill in.
- 2026-10-16 (UTC) · claude-code + D28b · Narrowed the Opus 5.5 and Sonnet 5.5 roles to the Pro routing the user chose: Opus for A and C packages only, Sonnet as the top implementer rung.
- 2026-10-16 (UTC) · claude-code + user request on model pruning · Added §4.1, a demote-and-promote rule driven by `oc.py`'s per-run result records, so that routing narrows to the models that keep passing on real tasks, as the user asked.
- 2026-10-16 (UTC) · claude-code + user model preference · Recorded the user's order (free, then Go, then Claude, with Claude kept for higher-level thinking) and named the free models each OpenCode row tries first, so that routing spends Claude only where judgment is the deliverable.
- 2026-10-16 (UTC) · claude-code + user correction on model choice · Replaced "free first" with the strongest suitable OpenCode model for the job, since the user said OpenCode usage is plentiful and some free models are weaker. Added head-to-head runs as evidence for the field record.
- 2026-10-16 (UTC) · claude-code + user prompt on Muse Spark 1.3 · Promoted Muse Spark 1.3, both variants, from O3 to O1 and made it the first choice for bookkeeping and a second choice for routine implementation, citing its bake-off results and its record in this repository rather than the privacy terms that had placed it in O3.
- 2026-10-16 (UTC) · claude-code + Go cap hit · Recorded that the Go cap is shared across the account and was hit after $5.01, mostly from expensive models. Added the cheap-implementer rule and two new run failure modes (a silent hang after a fatal provider error, and the paid title model), so later routing spends Go where it counts.
- 2026-10-16 (UTC) · claude-code + user preference, third statement · Set the order within the strong models: the intelligent free models' limits first, then cheap Go, then expensive Go for reviews, with Claude at the manager's discretion. The user wants tokens stretched as far as possible.
- 2026-10-16 (UTC) · claude-code + free-tier rate limits · Added the at-most-one-run-per-free-model rule after three concurrent DeepSeek Free runs and one Muse Spark Free run hit `Rate limit exceeded`.
- 2026-10-16 (UTC) · claude-code + free-tier rate limit correction · Corrected item 10: the free-tier limit is shared across the account, not set per model. Capped concurrency at three free runs in total, after four runs on four different free models failed at step 0.
- 2026-10-16 (UTC) · claude-code + user preference on LongCat and Cat Jam · Recorded that OpenCode lists both as unlimited for Go subscribers, the user's wish to use them heavily once proven, the rule that promotes them, and the unbenchmarked models now on the Go page.
- 2026-10-16 (UTC) · claude-code + rate-limit finding · Corrected item 11 the same afternoon: LongCat and Cat Jam also refuse under burst load (about eight and five concurrent runs), so they get a cap of three runs per model.
- 2026-10-16 (UTC) · claude-code + user correction on model routes · Replaced the burst-limit finding with its real cause: the refused runs used the free Zen IDs, which share the free tier's limit, while the `opencode-go/` IDs of the same models stayed available.
- 2026-10-16 (UTC) · claude-code + Go-route fix · Switched the routing table's Cat Jam and LongCat entries to their `opencode-go/` IDs, so the table no longer sends work to the rate-limited free route.
- 2026-10-16 (UTC) · claude-code + field record · Recorded the day's hand-run results per model and role, including Cat Jam's 68 of 70 held citations and DeepSeek's one stop-without-writing failure, so the next session applies §4.1 from evidence.
- 2026-10-16 (UTC) · claude-code + user routing rule · Recorded the user's rule that free Zen capacity goes only to strong models with a free Zen variant (Muse Spark, DeepSeek V4.1 Flash, GLM-5.3-Flash), while LongCat and Cat Jam always use their unlimited Go IDs.
- 2026-10-16 (UTC) · claude-code + free-tier reset time · Recorded the user's report that the free Zen limit takes about 12 hours to reset, and the 14:00 UTC repeat, so a manager routes to Go IDs after a hit instead of retrying free models.
- 2026-10-16 (UTC) · claude-code + manager handover · Recorded OpenCode 2.x as item 12, moved Scouting and the routine and bookkeeping routes off Cat Jam, and added the session field-record rows, so routing matches the versions and runs in use.
- 2026-10-16 (UTC) · claude-code/opus-5.5 + manager session 4 · Added item 13: Beam Free as the second uncapped Go model now that Cat Jam is gone, with its Go-only route and first runs, so a manager can keep slots busy through the Go cap.
- 2026-10-16 (UTC) · claude-code/opus-5.5 + user: Beam is Cat Jam renamed · Rewrote item 13 and the O2 tier entry so that Beam inherits Cat Jam's rules, tier place and field record, as the user directed; the earlier entry's "Cat Jam is gone" reading is superseded.
- 2026-10-16 (UTC) · claude-code/opus-5.5 + user: many free reviews · Added to item 13 the user's rule to run several independent reviews on Beam and LongCat and collate them before any paid review.
