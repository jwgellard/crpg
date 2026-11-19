# T100 — agent authoring benchmark (`tools/agents/authorbench`)

**Status: blind draft, 2026-11-19 (UTC), attempt 1.** Written by DeepSeek V4
Pro from the fact sheet (`tasks/T100-facts.md`, whose author's reports are not
evidence: every fact below was re-checked) and the T100 row. Another model is
drafting the same contract blind; this is one of the two drafts the Claude
architect adjudicates. Nothing here is approved.

## Task

Build an agent-authoring benchmark under `tools/`: the T100 row
(`docs/IMPLEMENTATION_PLAN.md:300`) asks for "campaign briefs of rising size
given to several models with only `crpgc` and the schemas", scoring validity,
playability (`crpgc run`), steps and errors, with a baseline on today's tools
run first and a re-run per T101 candidate command design, plus a "define a
small rule system" brief beside the campaign briefs. Nothing today measures
any of this: the closest tool is the bake-off harness
(`tools/agents/bakeoff/bakeoff.py`), which re-runs fixed scout/implement/review
fixtures and reuses `oc.py`, but has no authoring briefs, no `crpgc` scoring
and no design comparison.

T100 adds one std-library-only Python tool, `tools/agents/authorbench/`
(mirroring `bakeoff/`'s setup/run/score/all shape), its four briefs (three
campaign briefs of rising size set against the fact sheet's campaigns, and one
rule-system brief), and a `unittest` suite under `tools/tests/` that runs fixed
fake runs with a fake `crpgc` and never calls a model. It reuses `oc.py`'s
worker helpers exactly where `bakeoff.py` already does
(`tools/agents/bakeoff/bakeoff.py:30–34`, `:706–776`).

Two facts bound what "playable" can mean today, and this contract states them
rather than working around them (see Open questions Q1):
`crpgc run --ticks N --hash-every M [--seed S]`
(`crates/crpg-cli/src/main.rs:52`) calls `run_hash_sequence` once over an empty
world and never reads a campaign (`crates/crpg-cli/src/main.rs:2518–2519`;
`docs/architecture/crpg-cli.md:765–768`); and `crpgc validate <root> --json`
(`crates/crpg-cli/src/main.rs:35`) is the load-and-validate entry that exits 0
only when the campaign loads with no `Severity::Error` diagnostic
(`docs/architecture/crpg-cli.md:413–415`). So this draft scores **validity** as
`crpgc validate --json` exit 0, and **playability** as validity plus a
`crpgc run` smoke (exit 0) using the brief's fixed tick/hash arguments — the
`crpgc run` check is the row's literal command, kept even though it is
campaign-independent, and recorded as such.

## Parts

One part, **T100**, one area: the benchmark tool under `tools/`. It is not
S-set (`docs/IMPLEMENTATION_PLAN.md:86–98`): it touches no `World` field, tick
order, wire byte, schema version or workspace manifest, and it writes no
golden. It needs no build slot (M6-R, `docs/IMPLEMENTATION_PLAN.md:288–295`);
`deps.py` and `determinism.py` do not scan `tools/` (`tools/lint/deps.py:328`,
`tools/lint/deps.py:332–338` reads `crates/` only).

| Part | Area | Scope | Size (source + tests) | S-set | Review |
|---|---|---|---|---|---|
| T100 | `tools/` (no crate) | `authorbench.py`, its four briefs, `README.md`, the `unittest` suite + fixed fixtures | ~420 + ~380 | no | Claude reviewer reads the scorer against the score-record shape; no mutant run |

Order: one part. It may run beside any M4 package (no shared files).

### T100 owns
- `tools/agents/authorbench/authorbench.py` (new),
  `tools/agents/authorbench/README.md` (new),
  `tools/agents/authorbench/models.txt` (new)
- `tools/agents/authorbench/briefs/campaign-small/{brief.json,prompt.txt}`,
  `…/campaign-medium/{brief.json,prompt.txt}`,
  `…/campaign-large/{brief.json,prompt.txt}`,
  `…/ruleset-minimal/{brief.json,prompt.txt}` (all new)
- `tools/tests/test_authorbench.py` (new),
  `tools/tests/fixtures/authorbench/` (new: the fixed fake-run dirs and the
  fake `crpgc`, below)
- goldens: none new, none moved.

## Readings

Design decisions this draft makes. Each needs approval with the contract; the
architect adjudicates these against the other blind draft.

1. **The harness mirrors `bakeoff.py`'s shape and reuses `oc.py` helpers.**
   `bakeoff.py` already imports `oc` for `opencode_prefix`, `opencode_major`,
   `model_flag`, `wait_start_gap` and `run_worker` (`tools/agents/bakeoff/bakeoff.py:30–34`,
   `:706–776`; `tools/agents/oc.py:215`, `:251`, `:279`, `:518`, `:1070`). T100
   reuses the same five helpers to launch OpenCode workers and parse their JSONL
   log (`parse_log`, `tools/agents/bakeoff/bakeoff.py:107–139`), so the benchmark
   never re-implements launch, spend, or idle-watchdog behaviour. *Rejected:* a
   new dispatch layer in `authorbench.py` (drifts from `oc.py`'s dual-generation
   OpenCode support and its spend guard, `tools/agents/README.md:64–101`);
   reusing `bakeoff.py` itself (its fixture kinds are scout/implement/review,
   `tools/agents/bakeoff/bakeoff.py:647–662`; authoring is a fourth kind with a
   different scorer and a design dimension).
2. **Four briefs, one shape, all campaign-producing.** Every brief asks for a
   campaign directory `./campaign/` in the run dir, so every brief is scored by
   the same two `crpgc` checks. The rule-system brief is a campaign brief whose
   campaign uses a **novel** ruleset (not `minimal-d6` or `srd-lite`); it is
   not a standalone `rulesets/` package, because `crpgc validate` takes a
   campaign root (`crates/crpg-cli/src/main.rs:35`) and a bare ruleset package
   has no campaign to validate. Sizes are set against the fact sheet's
   campaigns: `combat_basic` is 15 files, `combat_srd` 18
   (`campaigns/fixtures/combat_basic/`, `campaigns/fixtures/combat_srd/`;
   verified 15 and 18 files), so small = 15, medium = 18, large = 25, and the
   rule-system brief = 15 files with a novel ruleset (`rulesets/minimal-d6/` is
   the reference: one `ruleset.json`, one ability, one outcome table, verified
   3 files plus `.gitattributes`). *Rejected:* separate non-campaign ruleset
   briefs (no `crpgc` command validates a bare package); a "tiny" 8-file brief
   (below any existing fixture, so the size ladder would not be "rising").
3. **The agent gets only `crpgc` and the brief; no repo, no `schemas/`.** The
   spec's literal test gives "only `crpgc schema creature` output"
   (`docs/CRPG_ENGINE_SPEC.md:1569`, `:2042`) and ranks the format by
   "machine-generatable by an LLM without training on a bespoke syntax"
   (`docs/CRPG_ENGINE_SPEC.md:421`). So the run dir holds only the brief and an
   empty working tree, the `crpgc` binary is put on `PATH` by the harness, and
   the brief tells the agent to obtain every schema via `crpgc schema <stem>`
   (17 stems, `crates/crpg-cli/src/main.rs:558–579`). The checked-in `schemas/`
   directory is not copied in. *Rejected:* copying `schemas/` into the run dir
   (the plan says "never point an agent at `schemas/`", `docs/AGENTIC_WORKFLOW_PLAN.md:330`);
   giving the agent the repository (it would read `combat_basic` and copy the
   answer).
4. **Validity is `crpgc validate --json` exit 0; playability is validity plus a
   `crpgc run` smoke.** `validate` is the one command that loads a campaign and
   reports positioned diagnostics with a `0/1/2` exit bucket
   (`docs/architecture/crpg-cli.md:16–19`, `:413–415`). `crpgc run` does not
   load the campaign (Reading from "Task"), so "playable" is
   `valid && run.rc == 0` where `run` is the brief's fixed
   `--ticks/--hash-every/--seed`. This keeps the row's literal `crpgc run`
   while never claiming it executes the campaign. *Rejected:* using
   `crpgc play --campaign <dir>` (interactive: the person chooses each move,
   `docs/architecture/crpg-cli.md:31–32`; no non-interactive mode to score);
   `crpgc demo` (needs a replay file, not an authored campaign);
   claiming `crpgc run` executes the campaign (it does not,
   `docs/architecture/crpg-cli.md:765–768`).
5. **The score record is one JSON object per run, keyed by a `design`.** The
   T100 row wants a baseline first and a re-run per T101 candidate command
   design, so every run record carries a `design` (`baseline` by default) and
   `score`/`compare` split on it. `compare` diffs each non-baseline design
   against `baseline` per brief and per model. *Rejected:* one scores file with
   no design key (a T101 re-run would overwrite the baseline); a separate
   `--design`-less `baseline.py` (two entry points for one scorer).
6. **Steps and errors come from the parsed event log, not the model's
   self-report.** `parse_log` already counts `step_finish` and `tool_use`
   events and records the first `error` event (`tools/agents/bakeoff/bakeoff.py:124–137`).
   The score record's `steps` is that `step_finish` count and `errors` is 1
   when any `error` event or the `err` string is present, else 0. *Rejected:*
   asking the model to count its own steps (untrusted); counting `tool_use` as
   steps (different quantity; both are recorded, only `step_finish` is scored).
7. **The sandbox is `bakeoff.py`'s `write_opencode_json`, edit-allowed, bash
   with no write outside the run dir.** The authoring worker must run `crpgc`,
   so it gets `edit: allow`, `bash: allow`, and every network/external write
   denied (`tools/agents/bakeoff/bakeoff.py:310–326`). The harness passes
   `--dir <run_dir>` (or the 2.x equivalent) exactly as `oc.py` does
   (`tools/agents/README.md:64–70`) and prepends the `crpgc` directory to
   `PATH`. *Rejected:* `edit: deny` (the agent cannot author); adding the repo
   to the sandbox (breaks "only `crpgc`", Reading 3).

## Interface

Stdlib only (`argparse`, `json`, `subprocess`, `pathlib`, `shutil`, `hashlib`,
`re`, `os`, `sys`, `time`, `concurrent.futures` — the `bakeoff.py` set,
`tools/agents/bakeoff/bakeoff.py:12–23`). No third-party dependency and no
network. `authorbench.py` imports `oc` from `tools/agents/` the way `bakeoff.py`
does (`tools/agents/bakeoff/bakeoff.py:30–34`).

### Command line

```text
python tools/agents/authorbench/authorbench.py setup <scratch>
python tools/agents/authorbench/authorbench.py run <scratch> --briefs <all|a,b>
        --models <m1,m2|@file> [--design NAME] [--variant V] [--repeat N]
        [--par N] [--out NAME] [--crpgc PATH]
python tools/agents/authorbench/authorbench.py score <scratch> [--design NAME]
        [--out NAME] [--json]
python tools/agents/authorbench/authorbench.py compare <scratch> [--out NAME]
        [--json]
python tools/agents/authorbench/authorbench.py all <scratch> [--design NAME]
        [--out NAME] [--variant V] [--repeat N] [--par N]
```

- `<scratch>` is a directory **outside** the repository (refused otherwise,
  exit 2, the `bakeoff.py` rule `tools/agents/bakeoff/bakeoff.py:98–104`).
- `--design` defaults to `baseline`; a T101 candidate re-run passes
  `--design <candidate>`. `--briefs` defaults to `all`. `--models` is required
  for `run` and `all`; `@tools/agents/authorbench/models.txt` is the default
  list when `--models` is omitted from `all`.
- `--crpgc` is the `crpgc` binary; default `$CRPGC`, else the build-tree
  `target/debug/crpgc` (`.exe` on Windows), else `crpgc` on `PATH`. A missing
  or non-executable `crpgc` is exit 2 `no-crpgc`.
- Exit codes, the `oc.py` convention (`tools/agents/README.md:3–6`): `0` pass,
  `1` a run failed (or `compare` found a regression), `2` usage or
  infrastructure.
- `run` prints one line per finished run (`<brief> <model> rc=<rc> <secs>s`,
  `tools/agents/bakeoff/bakeoff.py:763`) and returns 1 if any worker rc is
  non-zero. `score` and `compare` print verdict-first, then a table; `--json`
  prints one JSON array to stdout. `compare` returns 1 when any
  baseline-passing (brief, model) regresses in a candidate design (B-5).

### File layout (owned)

```text
tools/agents/authorbench/authorbench.py         the harness (setup/run/score/compare/all)
tools/agents/authorbench/README.md              usage guide (the bakeoff README shape)
tools/agents/authorbench/models.txt             default model list (one ID per line, `#` comments)
tools/agents/authorbench/briefs/<name>/brief.json   the brief's metadata (below)
tools/agents/authorbench/briefs/<name>/prompt.txt   the brief text (renamed to brief.md at setup)
tools/tests/test_authorbench.py                 the unittest suite
tools/tests/fixtures/authorbench/<name>/        fixed fake run dirs and logs (below)
tools/tests/fixtures/authorbench/fake_crpgc.py  the fake crpgc for tests (below)
```

### Brief format

`brief.json` (one object; unknown keys are a setup error, exit 2):

```json
{
  "name": "campaign-small",
  "kind": "campaign",
  "size": 15,
  "run": {"ticks": 64, "hash_every": 16, "seed": 1},
  "timeout_s": 1800,
  "novel_ruleset": false
}
```

- `kind`: `campaign` | `ruleset`. `size`: the target file count the brief
  states to the model (15, 18, 25, 15). `run`: the `crpgc run` arguments used
  to judge playability (the same values for every brief). `timeout_s`: the
  per-run budget. `novel_ruleset`: true only for `ruleset-minimal`.
- The four briefs: `campaign-small` (15 files, `combat_basic` scale: one hero
  and one hostile creature, one ability, one encounter, one ruleset, one
  outcome table), `campaign-medium` (18 files, `combat_srd` scale: adds effects
  and a second outcome table), `campaign-large` (25 files: adds an item, a
  second area, a second encounter), `ruleset-minimal` (15 files with a novel
  ruleset: one `ruleset.json`, one ability, one outcome table the model defines
  itself, plus the one-encounter campaign that uses it).

`prompt.txt` is the exact instruction handed to the model (one fixed text, the
same for every model and repeat). It must contain, verbatim where quoted:

- "Work only in the `campaign/` directory inside this directory."
- "You have only the `crpgc` command and its `schema` output. Do not read any
  other file in this directory or anywhere else."
- "Run `crpgc schema <stem>` for each document type you need, and `crpgc new
  <type> --slug <slug>` to scaffold one."
- "Check your work with `crpgc validate --json campaign/` and stop when it
  exits 0." (ruleset brief: "… and your campaign's ruleset is not `minimal-d6`
  or `srd-lite`.")
- "Then run `crpgc run --ticks 64 --hash-every 16 --seed 1` and report its last
  line."
- The size line: "Aim for about `<size>` files." (15, 18, 25, 15).

At `setup`, `prompt.txt` is copied into the run dir and renamed `brief.md`
(the `bakeoff.py` rule that `.txt` files are renamed to `.md` so no agent in
this repository loads them as instructions, `tools/agents/bakeoff/README.md:31–32`),
and the prompt is passed to the model as the OpenCode prompt: `Read brief.md
and do what it says.` The brief's text is not placed in the repository as
`.md`.

### Score record shape

`score` writes one JSON array to `<scratch>/<out>/<design>.scores.json`
(the `scores.json` path of `bakeoff.py`, `tools/agents/bakeoff/bakeoff.py:883–885`,
extended with the design key). Each element:

```json
{
  "brief": "campaign-small",
  "kind": "campaign",
  "design": "baseline",
  "model": "opencode-go/deepseek-v4-pro",
  "repeat": 0,
  "variant": null,
  "rc": 0,
  "secs": 123.4,
  "steps": 12,
  "tools": 20,
  "tok": {"input": 120, "output": 120, "reasoning": 0, "cache_read": 60},
  "cost": 0.12,
  "err": "",
  "valid": true,
  "diagnostics": [],
  "run": {"rc": 0, "ticks": 64, "hash_every": 16, "seed": 1, "samples": 4},
  "novel_ruleset": false,
  "score": {"valid": true, "playable": true, "steps": 12, "errors": 0}
}
```

Field rules (normative):
- `valid`: `crpgc validate --json <dir>/campaign` exited 0.
- `diagnostics`: the parsed JSON array from that `--json` output (each element
  the data-owned `Diagnostic` shape — `file`, `pointer`, `severity`, `code`,
  `message`, `suggested_fix`, `crates/crpg-data/src/validation.rs:175–188`).
  Empty (`[]`) when valid.
- `run`: the `crpgc run` smoke; `rc` its exit code, `samples` the count of
  `<k> <64-hex>\n` lines on stdout (for ticks 64 / hash_every 16 that is 4:
  samples at 16, 32, 48, 64, `docs/architecture/crpg-cli.md:766–768`).
- `novel_ruleset`: `kind == "ruleset"` and every `campaign/rulesets/*.json`
  `slug` differs from `minimal-d6` and `srd-lite`; always `false` for
  `kind == "campaign"`.
- `score`: computed — `valid` = `valid`; `playable` = `valid && run.rc == 0`;
  `steps` = `steps`; `errors` = 1 if `err` is non-empty, else 0.
- `rc`, `secs`, `steps`, `tools`, `tok`, `cost`, `err` come from `parse_log`
  and the run meta exactly as `bakeoff.py` fills them
  (`tools/agents/bakeoff/bakeoff.py:868–881`).

`compare` reads `<out>/<design>.scores.json` for every design present and emits
one object per (brief, model, candidate) with `baseline` and `candidate`
`{valid, playable, steps, errors}` plus `regressed` (B-5).

## Behaviour

Normative. Each item names the tests that prove it. "Refused" means the stated
exit and no state change. **First failure wins** in every ordered list.

B-1. **`setup <scratch>`.** Refuses a scratch inside the repository (exit 2).
Otherwise it creates `<scratch>/tpl/<brief>/` for each brief by copying the
brief's directory (not `brief.json`, not `prompt.txt`), renames any `*.txt` to
`*.md` there, and writes nothing else. A brief whose `brief.json` has an
unknown key, a missing `run`/`timeout_s`/`kind`, or a `size`/`kind`/`run`
value outside the four briefs' set is a setup error (exit 2), named with the
brief and the bad key. *Tests:* T1.
B-2. **`run <scratch>`.** Refuses a scratch inside the repository (exit 2);
refuses a missing template (exit 2, "run setup first"); refuses an unknown
brief name or a missing `--models` (exit 2); refuses a missing `crpgc` (exit 2
`no-crpgc`). For each (brief, model, repeat) it copies `tpl/<brief>` to
`<scratch>/<out>/runs/<brief>/<model>[/r<repeat>]`, writes the `opencode.json`
sandbox (Reading 7), invokes the model via `oc.run_worker` with the brief
prompt, and writes a `.meta.json` beside the log carrying the same keys
`bakeoff.py` writes (`tools/agents/bakeoff/bakeoff.py:751–762`) plus `design`.
The `crpgc` directory is prepended to `PATH` for the worker. A worker rc that
is not 0 makes the whole `run` exit 1. *Tests:* T2.
B-3. **`score <scratch>`.** Reads every `<out>/*.meta.json`, re-parses each
log with `parse_log`, and for each run: runs `crpgc validate --json
<run_dir>/campaign` (capturing stdout and rc), then runs `crpgc run --ticks 64
--hash-every 16 --seed 1` (capturing stdout and rc) **always** — the row's
literal smoke, campaign-independent — and sets `playable = valid && run.rc ==
0`, so a false `valid` keeps `playable` false. `samples` is the count of lines
matching `^[0-9]+ [0-9a-f]{64}$` on `crpgc run` stdout. It computes
`novel_ruleset` per the Interface rule and writes the array to
`<out>/<design>.scores.json`. A run directory with no `campaign/` scores
`valid: false` and `diagnostics: []` (validate cannot load); `err` is never
written by the harness, only copied from `parse_log`. `score` never exits 1
for a failed run (it records, not judges); it exits 2 only for a usage or a
missing/unknown meta or brief. *Tests:* T3, T4.
B-4. **Determinism.** Two `score` calls over the same `<scratch>` and `--design`
write byte-identical `<design>.scores.json`. The harness draws no clock or
random value of its own; `secs` and `rc` come from the run meta, and the model
is never re-invoked by `score` or `compare`. *Tests:* T5.
B-5. **`compare <scratch>`.** For each design other than `baseline` present,
and each (brief, model) present in both `baseline` and that design, it emits
`{brief, model, design, baseline, candidate, regressed}` where `regressed` is
true iff the candidate is strictly worse by this order: `valid` false when
baseline true; else `playable` false when baseline true; else `steps` greater;
else `errors` greater. It returns 1 when any `regressed` is true, else 0; a
missing `baseline.scores.json` is exit 2 `no-baseline`. *Tests:* T6.
B-6. **Nothing is charged to the model that is the harness's fault.** A
missing `crpgc` (exit 2 `no-crpgc`), a scratch-inside-repo refusal, a missing
template, and a missing brief are all exit 2 and are never written as a run
record or a model failure (the `oc.py` non-fault causes rule,
`tools/agents/README.md:156–158`). *Tests:* T2.
B-7. **The fake `crpgc` is the only model-free stand-in.** The test suite
invokes `authorbench.py` only with `$CRPGC` pointing at
`tools/tests/fixtures/authorbench/fake_crpgc.py`, never the real `crpgc`,
never `opencode`, never the network. *Tests:* all.

## Errors

| Condition | Exit / value |
|---|---|
| scratch inside the repository (`setup`/`run`/`score`/`compare`/`all`) | exit 2, one line `setup: scratch directory is inside the repository; refused` (the `bakeoff.py` text, `tools/agents/bakeoff/bakeoff.py:269`) |
| unknown or malformed brief (`setup`) | exit 2, `setup: brief <name>: <key> is …` |
| unknown brief name (`run`) | exit 2, `run: unknown brief <name>` |
| missing template (`run`) | exit 2, `run: template missing for <name>; run setup first` |
| missing `--models` (`run`) | exit 2, `usage` |
| missing `crpgc` (`run`/`score`) | exit 2, `no-crpgc: cannot run <path>` |
| unknown meta or brief (`score`) | exit 2, `score: unknown brief <name>` |
| missing `baseline.scores.json` (`compare`) | exit 2, `compare: no baseline scores (<path>)` |
| any worker rc ≠ 0 (`run`) | exit 1 |
| any `regressed` true (`compare`) | exit 1 |

A run whose `campaign/` directory is absent is not an error: it scores
`valid: false`, `diagnostics: []` (B-3), and is compared like any other run.

## Tests

`python -m unittest discover -s tools/tests -p "test_authorbench.py" -v` runs
under 60 seconds, never the real `crpgc`/`opencode`/network (B-7). Every case
names a control that changes one input and flips the expectation, and a "fails
if". The fixed log is built by the test, not committed: 12 `step_finish` lines,
each `{"type":"step_finish","part":{"tokens":{"input":10,"output":10,`
`"reasoning":0,"cache":{"read":5}},"cost":0.01}}`, and 20
`{"type":"tool_use","part":{}}` lines, no `error` line — so `parse_log` returns
`steps == 12`, `tools == 20`, `err == ""`, `cost == 0.12`
(`tools/agents/bakeoff/bakeoff.py:124–137`). The fixture run dir holds a
`campaign/` whose `campaign.json` is a fixed minimal campaign written by the
test (any fixed bytes not containing the substring `"broken"`).

The fake `crpgc` is `tools/tests/fixtures/authorbench/fake_crpgc.py`:
`validate --json <dir>` reads `<dir>/campaign.json`; if the text contains
`"broken"` it prints exactly
`[{"file": "campaign.json", "pointer": "/name", "severity": "error", "code":`
` "missing_field", "message": "campaign name is required", "suggested_fix":`
` "add a name"}]` and exits 1; if the file is absent it prints `[]` and exits
1; otherwise it prints `[]` and exits 0. `run --ticks N --hash-every M --seed
S` prints one line `<k> ` + 64 `a`s for `k` in `M, 2M, … ≤ N` and exits 0 (so
ticks 64 / hash_every 16 prints 4 lines: 16, 32, 48, 64).

- **T1 `setup_builds_templates_and_refuses_bad_brief`.** `setup` on a temp
  scratch outside the repo: exit 0, and each of the four briefs has
  `<tpl>/<name>/brief.md` present and no `prompt.txt` left. *Control:* a
  scratch equal to the repository root → exit 2 with the "inside the
  repository" line; a `brief.json` with its `run` key deleted → exit 2 naming
  the brief. *Fails if:* a template keeps `prompt.txt`, misses `brief.md`, or
  either refusal does not exit 2.
- **T2 `run_refusals_and_happy_path`.** With `$CRPGC` = the fake `crpgc` and
  `$CRPG_OPENCODE` = a fake that writes the fixed 32-line log and exits 0:
  `run` on a temp scratch, one brief, one model, `--design baseline` → exit 0,
  and `<out>/runs/<brief>/<model>/` holds the `campaign/` copy, the log, and a
  `.meta.json` whose `design` is `"baseline"`. *Control:* a scratch inside the
  repo → exit 2; `--models` omitted → exit 2 `usage`; `$CRPGC` unset and
  `--crpgc` pointing at a missing file → exit 2 `no-crpgc`. *Fails if:* the
  happy path exits non-zero, the meta lacks `design`, or a refusal exits 0.
- **T3 `score_valid_campaign`.** Run `score` over the happy-path scratch from
  T2. The one record equals, field for field:
  `valid == true`, `diagnostics == []`, `run == {"rc": 0, "ticks": 64,`
  `"hash_every": 16, "seed": 1, "samples": 4}`, `novel_ruleset == false`,
  `steps == 12`, `tools == 20`, `errors == 0`, `score == {"valid": true,`
  `"playable": true, "steps": 12, "errors": 0}`. *Fails if:* any of those
  fields differs, or `playable` is false.
- **T4 `score_broken_campaign`** (control: `campaign.json` now contains
  `"broken"`, one input changed from T3). `valid == false`,
  `diagnostics == [{"file": "campaign.json", "pointer": "/name", "severity":`
  ` "error", "code": "missing_field", "message": "campaign name is required",`
  ` "suggested_fix": "add a name"}]`, `run.rc == 0`, `run.samples == 4`
  (`run` still ran), `score.playable == false`. *Fails if:* `playable` is true,
  `run` was skipped (`run.samples == 0`), or `diagnostics` has any other count.
- **T5 `score_counts_an_error_event`** (control: the log gains one
  `{"type":"error","error":{"message":"boom"}}` line, one input changed from
  T3). `score.errors == 1` and `score.valid == true`, `score.playable == true`
  (the campaign is still valid; the error is the model's, not the campaign's).
  *Fails if:* `errors` stays 0, or validity flips.
- **T6 `score_missing_campaign`** (control: the `campaign/` directory is
  removed, one input changed from T3). `valid == false`, `diagnostics == []`,
  `run.rc == 0`, `run.samples == 4`, `score.playable == false`,
  `score.errors == 0` (the harness wrote no `err`). *Fails if:* the harness
  crashes on the missing directory, or `valid` is true.
- **T7 `compare_flags_a_regression`.** A `baseline` scores file (T3's record)
  and a `candidate` scores file differing only in `steps == 15`: `compare`
  emits `regressed == true` and exits 1. *Control:* a second `candidate` file
  with `steps == 12` and `errors == 0` → `regressed == false` and exit 0.
  *Fails if:* a `valid`/`playable` flip does not regress, a step decrease
  regresses, or the exit code is wrong.
- **T8 `score_is_deterministic`.** Two `score` calls over the same scratch and
  `--design` write byte-identical `<design>.scores.json`. *Fails if:* the two
  files differ.

## What must not change

- Every existing file under `tools/` byte for byte, except the owned new files:
  `tools/agents/oc.py`, `tools/agents/bakeoff/` (script, fixtures, README,
  models.txt), `tools/agents/{brief,recall,lost,fmt_one,handover,context_meter,
  protect,verify_task}.py`, `tools/lint/` and its self-tests, `tools/tests/`
  (existing `test_*.py`, `fake_opencode.py`, fixtures), `tools/preflight.{sh,ps1}`,
  `tools/README.md`.
- Every Rust crate and its tests, every golden, every schema under `schemas/`,
  every campaign under `campaigns/` and `rulesets/` (the benchmark reads
  `campaigns/` and `rulesets/` as size references only, never writes them).
- `crates/crpg-contracts`, `rust-toolchain.toml`, `Cargo.toml`, `Cargo.lock`,
  `.github/workflows`, `opencode.json`, `.opencode/` (none are owned).
- The behaviour of `crpgc` (the benchmark invokes it, it does not change it);
  `tools/lint/deps.py` and `tools/lint/determinism.py` still pass because they
  do not scan `tools/` (`tools/lint/deps.py:328`, `:332–338`).

## Dependency edges

No Rust dependency edge: `authorbench.py` is std-library Python and imports
only `oc` from `tools/agents/` (`tools/agents/bakeoff/bakeoff.py:30–34`, the
same pattern). No `[dependencies]`/`[dev-dependencies]`/`[build-dependencies]`
entry is added, `Cargo.lock` is untouched, and `tools/lint/deps.py` `ALLOWED`
is unchanged (it enumerates workspace crates only, `tools/lint/deps.py:46–72`,
and never reads `tools/`). No third-party package is imported, so the
"no new dependency without approval" rule (`AGENTS.md:52`) needs no user
question; the std-only alternative is the design, not a fallback.

## Out of scope

- Any change to `crpgc`, `crpg-data`, the schemas, or the campaigns/rulesets.
- Running the benchmark for real in this task (the baseline and each T101
  re-run are separate manager actions once `crpgc` is built); T100 only
  builds the tool and proves it on fixed fakes.
- The T101 ADR and its candidate command designs: T100 records a `design` and
  compares, but does not design or judge the candidates.
- Model selection and routing policy (`docs/agents/MODELS.md` §4) — the
  benchmark consumes `models.txt`, it does not decide it.
- Any scoring beyond validity, the `crpgc run` smoke, steps and errors
  (no "did the dialogue branch correctly", no hidden reference campaign, no
  human preference ranking).

## Open questions

Each with a recommendation, for the architect and the user.

1. **The T100 row names `crpgc run` as the playability check, but `crpgc run`
   never loads a campaign** (`crates/crpg-cli/src/main.rs:2518–2519`,
   `docs/architecture/crpg-cli.md:765–768`). *Recommend:* accept this draft's
   reading — "playable" = `valid && crpgc run` exit 0, with the
   campaign-independence stated in the record — and let T101 decide whether a
   campaign-aware `crpgc run --campaign` should later replace the smoke.
2. **Is a 15/18/25/15 size ladder the right "rising size"?** *Recommend:* yes;
   it is anchored to the only two real campaigns (15 and 18 files) and stays
   inside the fact sheet's brief budget.
3. **Should the rule-system brief also require the novel ruleset to be
   *playable*, not just valid?** *Recommend:* validity only for this draft
   (playability of a novel ruleset needs `crpgc run --campaign`, Q1); the
   `novel_ruleset` flag is recorded so T101 can tighten it.
4. **May `run` re-enter the model on a non-zero worker rc (retry)?** The
   `bakeoff.py` "database is locked" retry is the only retry there
   (`tools/agents/bakeoff/bakeoff.py:739–749`). *Recommend:* no retry for a
   non-zero rc (an authoring failure is data, not infra); keep only the
   locked-DB retry.
5. **Is `errors` = "any error event" too coarse?** *Recommend:* keep it coarse
   for the baseline; the T101 comparison needs a stable scalar, and the full
   `err` string is already recorded for later analysis.

## Agent log

- 2026-11-19 (UTC) · opencode-go/deepseek-v4-pro + T100 draft a · Drafted the blind contract for the T100 agent authoring benchmark: a std-library-only `tools/agents/authorbench/` harness that reuses `oc.py`/`bakeoff.py` to give several models four campaign briefs (three rising sizes plus one novel-ruleset brief) with only `crpgc`, and scores validity (`crpgc validate --json`), the `crpgc run` smoke, steps and errors with a `design` key so T101 candidates re-run against a baseline.
