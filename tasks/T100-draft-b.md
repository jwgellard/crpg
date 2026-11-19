# T100 — Agent authoring benchmark under `tools/` (draft B, blind)

**Status: blind draft, 2026-11-19 (UTC). Not approved. Nothing here is approved.**

## Task
Build the T100 agent authoring benchmark as a stdlib-only Python package
under `tools/`. It hands campaign briefs of rising size — plus one
"define a small rule system" brief — to several models with only `crpgc`
and the schemas, then scores each run on validity
(`crpgc validate --json`), playability (`crpgc run`), steps and errors.
It runs a baseline on today's tools first and re-runs on each candidate
command design, so T101 can compare authoring UX proposals against the
same numbers (`docs/IMPLEMENTATION_PLAN.md:300`–`docs/IMPLEMENTATION_PLAN.md:301`;
M6-R needs no build slot, `:288`, `:295`, `:416`).
"Playable" here means exactly what today can check headlessly: the
authored campaign validates clean AND the bounded hash harness completes
(`crates/crpg-cli/src/main.rs:2518`–`crates/crpg-cli/src/main.rs:2519`
runs `run_hash_sequence` over an empty world, not the campaign — Reading 4).
Model runs are launched through the existing dispatcher
(`tools/agents/oc.py`, `tools/agents/README.md:9`–`tools/agents/README.md:28`)
with the benchmark brief supplied as `--brief`; scoring, records and
comparison are new. Tests are Python `unittest` under `tools/` with fixed
fake runs: no network, no model calls
(`tools/agents/README.md:196`).

## Parts
One part, **T100**, in `tools/`. Not S-set (`docs/IMPLEMENTATION_PLAN.md:86`–`docs/IMPLEMENTATION_PLAN.md:98`):
no `World` field, tick order, wire byte, checkpoint, history version,
schema or workspace manifest is touched; no Rust crate is touched at all.

Order: one part; nothing to serialize.

### T100 owns
- `tools/author_bench/run_bench.py` (new; the only executable entry)
- `tools/author_bench/score.py` (new; validity/playability scoring over
  captured `crpgc` outputs — no subprocess, no network)
- `tools/author_bench/briefs/b1_single_file.md` (new)
- `tools/author_bench/briefs/b2_small_campaign.md` (new)
- `tools/author_bench/briefs/b3_full_campaign.md` (new)
- `tools/author_bench/briefs/b4_rule_system.md` (new)
- `tools/tests/test_author_bench.py` (new; the only test file)
- goldens: none.

## Readings
Design decisions this contract makes. Each needs approval with the contract.

1. **Bench lives in `tools/author_bench/`, tests in `tools/tests/`.**
   `tools/` holds only `lint/`, `preflight.sh`/`preflight.ps1`, `tests/`
   (`tools/README.md:1`–`tools/README.md:9`); agent dispatch already lives
   one level down in `tools/agents/` with its tests in `tools/tests/`
   (`tools/agents/README.md:189`–`tools/agents/README.md:200`), so a second
   package plus its tests in the same two places copies the house shape.
   *Rejected:* a new top-level directory (a second tool root for one
   benchmark); tests beside the package (nothing else under `tools/`
   does that).
2. **Model runs go through `tools/agents/oc.py run --brief`.**
   `run` already prepares a warm slot and branch, starts the worker,
   runs the verifier and writes a result record with `steps`
   (`tools/agents/README.md:9`–`tools/agents/README.md:54`;
   `tools/agents/oc.py:2404`, `tools/agents/oc.py:37`); the benchmark
   brief is passed as `--brief` (`tools/agents/README.md:122`–`tools/agents/README.md:126`),
   which the dispatcher already resolves per run. The harness never
   spawns a model itself.
   *Rejected:* a bespoke launcher (re-implements slot auto-pick, claims,
   and records); calling the model API from `score.py` (network in
   scoring; `tools/agents/brief.py:8` sets the stdlib/no-network norm).
3. **The agent gets only `crpgc`, the schemas, and its brief.**
   The T100 row says "with only `crpgc` and the schemas"
   (`docs/IMPLEMENTATION_PLAN.md:300`); the brief therefore allows
   exactly the eleven subcommands
   (`crates/crpg-cli/src/main.rs:255`–`crates/crpg-cli/src/main.rs:271`:
   `replay validate migrate new schema explain fmt lock run demo play`),
   the seventeen `schema` stems
   (`crates/crpg-cli/src/main.rs:558`–`crates/crpg-cli/src/main.rs:579`),
   and writing under one output directory. Nothing else in the repo is
   readable from the slot brief.
   *Rejected:* full-repo access (measures editor skill, not tool UX).
4. **"Playable" = validates clean AND `crpgc run` exits 0.**
   `run` is a bounded hash harness over an empty world
   (`crates/crpg-cli/src/main.rs:2518`–`crates/crpg-cli/src/main.rs:2519`;
   prints `<tick> <64-hex>` per completed multiple); it never loads the
   authored campaign, so it cannot mean "the campaign executes". The
   honest proxy available through `crpgc` alone is the conjunction:
   `validate --json` exit 0 with zero diagnostics plus `run` exit 0
   with the expected sample count. Encounter briefs additionally require
   the scripted `perform_action` check to stay out: it is not a `crpgc`
   command and would make T101 re-runs incomparable.
   *Rejected:* `crpgc play`/`demo` as the playability gate (interactive
   viewers over testkit playback, not scriptable pass/fail);
   testkit-driven combat (not runnable through `crpgc`, breaks the
   baseline-vs-candidate comparison).
5. **Brief sizes are pinned against today's fixtures.**
   B1 matches the literal LLM acceptance shape (one creature/item/
   dialogue/quest file, as `crates/crpg-cli/tests/scaffolding.rs:507`,
   `:530`, `:612` install and require to `validate --json` clean);
   B2 matches `campaigns/fixtures/combat_basic` (15 files, 262 lines);
   B3 matches `campaigns/fixtures/combat_srd` (18 files, 408 lines);
   B4 matches `rulesets/minimal-d6` (4 files, 89 lines) with the
   `rulesets/srd-lite` (7 files, 230 lines) shape as its stretch goal.
   *Rejected:* invented sizes (unmoored from what the tools chew today).
6. **Scoring is a pure function over captured outputs.**
   `score.py` takes recorded exit codes, stdout bytes and the result
   record — never a live process — so tests use fixed fakes and the
   baseline record re-scores byte-identically years later.
   *Rejected:* scoring inside the runner's subprocess loop (untestable
   without a model).
7. **Steps and errors come from the `oc.py` result record plus the
   captured `crpgc` transcript.** `steps` is the record's `steps` field
   (shape in `tools/agents/oc.py:1747`); `errors` is the ordered list of
   nonzero-exit `crpgc` invocations (command, exit, last stderr line)
   plus every `validate --json` diagnostic message. No new counting.
   *Rejected:* wall-clock-derived "effort" (unreproducible across models).

## Interface
Stdlib only, Python 3.11+ (the `oc.py` norm,
`tools/agents/README.md:12`). No network, no model calls, no new
dependency (`AGENTS.md:52`). Exit codes follow the `oc.py` buckets
(`tools/agents/README.md:5`–`tools/agents/README.md:7`): 0 scored,
1 a check failed, 2 usage or infrastructure.

### Command line
```text
python tools/author_bench/run_bench.py --models M,... --out OUT
    [--briefs DIR] [--ticks N] [--hash-every M] [--seed S]
    [--timeout SEC] [--crpgc PATH]
python tools/author_bench/run_bench.py --compare BASE.json CAND.json
    [--out OUT]
```
`--models` is one `oc.py` `--model` ID per run
(`tools/agents/README.md:15`–`tools/agents/README.md:28`); `--out` is
the directory receiving one score record per (model, brief) plus
`summary.json`. `--ticks`/`--hash-every`/`--seed` default to
`100`/`50`/`0` and are passed to every `crpgc run` invocation, whose
grammar is `crpgc run --ticks N --hash-every M [--seed S]`
(`crates/crpg-cli/src/main.rs:52`). `--crpgc` defaults to
`env!("CARGO_BIN_EXE_crpgc")`-built `crpgc` on `PATH`. `--compare`
re-scores nothing: it diffs two record files.

### File layout
```text
tools/author_bench/run_bench.py   # launch (via oc.py) + collect + write records
tools/author_bench/score.py       # pure scoring: validity, playability, roll-up
tools/author_bench/briefs/b1_single_file.md
tools/author_bench/briefs/b2_small_campaign.md
tools/author_bench/briefs/b3_full_campaign.md
tools/author_bench/briefs/b4_rule_system.md
tools/tests/test_author_bench.py  # unittest, fixed fakes only
```

### Brief format
Each brief is a self-contained agent brief: goal, the allowed commands
(the eleven subcommands of
`crates/crpg-cli/src/main.rs:260`–`crates/crpg-cli/src/main.rs:270`),
the allowed reads (`crpgc schema <stem>` for the seventeen stems of
`crates/crpg-cli/src/main.rs:558`–`crates/crpg-cli/src/main.rs:579`,
plus the checked-in `schemas/` files), the single output directory,
and the acceptance (its files pass `crpgc validate --json`, where
`crpgc validate <campaign-root> [--json]` is
`crates/crpg-cli/src/main.rs:37`). B1 asks for one document of a named
scaffold type (`crpgc new <type> --slug <s> --id <id>`,
`crates/crpg-cli/src/main.rs:47`); B2 asks for a `combat_basic`-scale
campaign (hero, goblin, one strike ability, one encounter); B3 asks
for a `combat_srd`-scale campaign (two abilities, one effect, two
outcome tables); B4 asks for a `minimal-d6`-scale rule system
(ruleset document, one ability, one outcome table).

### Score record (one JSON object per model × brief)
```json
{
  "benchmark": "T100",
  "model": "<oc.py model id>",
  "brief": "b1_single_file",
  "crpgc_version": "<`crpgc --version` or `CARGO_PKG_VERSION`>",
  "valid": true,
  "validate_exit": 0,
  "diagnostics": 0,
  "playable": true,
  "run_exit": 0,
  "run_samples": 2,
  "steps": 7,
  "errors": [{"command": "crpgc validate", "exit": 1, "line": "<last stderr line>" }],
  "wall_s": 12.5,
  "score": 100
}
```
`score` is 0–100: 50 for `valid`, 30 for `playable`, 20 scaled by
`max(0, 1 - errors / max(1, steps))`, rounded half up. `summary.json`
holds the records array plus `{"baseline_of": "<path>"}` when this run
is the baseline. `--compare` prints one line per brief:
`model brief base_score cand_score delta verdict`, where verdict is
`better|same|worse` on the score delta.

## Behaviour
Normative. Each item names the tests that prove it. "Run" below is one
(model, brief) pair; "record" is its score-record object.

- B1. **Layout and stdlib.** The part owns exactly the files of "T100
  owns"; every `tools/author_bench/*.py` imports stdlib only and makes
  no socket, model-API or subprocess call from `score.py`.
  *Tests:* F1, F8.
- B2. **Baseline first.** A scoring run with no `--compare` writes one
  record per (model, brief) plus `summary.json` carrying
  `"baseline_of"`; a later `--compare BASE CAND` run reads both files
  and prints one verdict line per brief. *Tests:* F2, F9.
- B3. **Launch reuses `oc.py`.** Each run is started as
  `python tools/agents/oc.py run --task T100 --role grunt --model M
  --brief tools/author_bench/briefs/<b>.md`, inheriting slot auto-pick,
  the occupied-slot guard, attempt claims and result records
  (`tools/agents/README.md:33`–`tools/agents/README.md:54`).
  *Tests:* F3.
- B4. **Sandbox allow-list.** The brief given to the model permits only
  the eleven `crpgc` subcommands, `crpgc schema` stems and the run's
  output directory (Reading 3); the harness records the transcript of
  every `crpgc` invocation (command, exit, last stderr line).
  *Tests:* F4.
- B5. **Validity.** `valid` is true iff `crpgc validate --json` on the
  run's output exits 0 AND its parsed diagnostics list is empty
  (`crates/crpg-cli/src/main.rs:1619`–`crates/crpg-cli/src/main.rs:1639`
  renders diagnostics as JSON when `--json`); `diagnostics` is the list
  length, `validate_exit` the exit. Unparsable stdout counts as
  invalid with one synthetic diagnostic. *Tests:* F5.
- B6. **Playability.** `playable` is true iff `valid` AND
  `crpgc run --ticks N --hash-every M` exits 0 AND stdout holds exactly
  the sample lines for ticks M, 2M, … ≤ N, each
  `<tick> <64 lowercase hex>`
  (`crates/crpg-cli/src/main.rs:2518`–`crates/crpg-cli/src/main.rs:2531`);
  `run_samples` is the line count, `run_exit` the exit. With defaults
  N=100, M=50 the expected lines are 2 (ticks 50 and 100). *Tests:* F6.
- B7. **Steps and errors.** `steps` is the `oc.py` result record's
  step count for the run; `errors` lists every nonzero-exit `crpgc`
  invocation plus every `validate --json` diagnostic message, in
  order. *Tests:* F7.
- B8. **Score arithmetic.** `score = 50·valid + 30·playable +
  round(20·max(0, 1 − errors/max(1, steps)))`, an integer 0–100; a
  valid, playable, error-free run scores 100. *Tests:* F8.
- B9. **Compare verdicts.** `--compare` verdict per brief is `better`
  when `cand − base > 0`, `same` when 0, `worse` when < 0; it exits 1
  when any brief is `worse`, else 0. *Tests:* F9.
- B10. **No model, no network in tests.** The suite passes with the
  network unavailable and no model configured; every subprocess is a
  checked-in fake. *Tests:* F10 (plus every F-case, which uses fakes).

## Errors
Every variant, and when. Exit buckets follow
`tools/agents/README.md:5`–`tools/agents/README.md:7`.

| Value | When |
|---|---|
| exit 2 `usage: --models and --out are required` | scoring run without `--models` or `--out` |
| exit 2 `usage: --compare takes exactly two record files` | `--compare` with other than two paths |
| exit 2 `no-brief: <path> missing` | a `briefs/` file absent (mirrors `oc.py`'s `no-brief`, `tools/agents/README.md:125`–`tools/agents/README.md:126`) |
| exit 2 `no-crpgc` | the `crpgc` binary missing or not executable |
| exit 1 `worse: <brief>` (one line per regressed brief) | `--compare` finds any brief verdict `worse` |
| record with `valid: false`, one synthetic diagnostic `unparsable validate output` | `validate --json` stdout is not JSON |
| record with `playable: false` | `valid` false, `run_exit` nonzero, or sample lines malformed |

No new exit bucket beyond 0/1/2; `oc.py`'s own causes (`quota`,
`402`, `locked`, `spend`, `verify-timeout`, `bad-ref`, `empty-diff`
never count against the model,
`tools/agents/README.md:153`–`tools/agents/README.md:158`) propagate
unchanged and are not rescored.

## Tests
In `tools/tests/test_author_bench.py`, stdlib `unittest`, fixed fakes
only: a `fake_crpgc.py` (answers `validate --json` and `run` from
argv-keyed canned bytes) and canned `oc.py` result-record dicts. No
network, no model, no repo write outside `tempfile`. Every case has a
control changing one input and a "fails if"; expected values below are
hand-computed from the canned bytes.

- F1 `stdlib_only`. Imports `run_bench` and `score`, asserts no module
  source contains `socket`, `urllib`, `http`, `openai` or `subprocess`
  inside `score.py` (grep over the two files). *Control:* the same grep
  over `run_bench.py` allows `subprocess` (the launcher needs it).
  *Fails if:* `score.py` shells out or touches the network.
- F2 `baseline_writes_records_and_summary`. Fake: models `m-a,m-b`,
  briefs `b1,b2`, validate `[]` exit 0, run two sample lines exit 0,
  steps 4, no errors. Expects 4 records each `score == 100`
  (50 + 30 + round(20·1)) and `summary.json` with 4 entries and a
  `baseline_of` key. *Control:* rerun with `--compare` on the same
  file twice: four `same` lines, exit 0. *Fails if:* a record is
  missing or the summary lacks the array.
- F3 `launch_argv_uses_oc_run_brief`. Fake launcher records argv;
  expects
  `oc.py run --task T100 --role grunt --model m-a --brief
  tools/author_bench/briefs/b1_single_file.md` as a subsequence.
  *Control:* brief `b4_rule_system.md` changes only the `--brief`
  path. *Fails if:* the harness invents its own spawn flags.
- F4 `brief_allow_lists_only_crpgc`. Reads each of the four briefs,
  asserts each names `crpgc` and `schema`, and none names `cargo`,
  `git push`, `curl` or a second crate path. *Control:* a scratch copy
  with one added `cargo test` line fails the same assertion. *Fails
  if:* a brief grants the model the repo.
- F5 `validity_empty_diagnostics`. Canned `validate --json` stdout
  `[]`, exit 0 → `valid == True`, `diagnostics == 0`,
  `validate_exit == 0`. *Control:* stdout
  `[{"severity":"error","message":"missing ruleset"}]`, exit 1 →
  `valid == False`, `diagnostics == 1`. *Control 2:* stdout `not
  json`, exit 0 → `valid == False` with the single synthetic
  `unparsable validate output` diagnostic. *Fails if:* exit 0 alone
  decides validity.
- F6 `playability_two_samples`. Canned run stdout
  `50 <"ab"*32>\n100 <"cd"*32>\n`, exit 0, with F5's valid case →
  `playable == True`, `run_samples == 2`. *Control:* stdout with only
  the `50` line (hash-every changed to 100 in one input) →
  `run_samples == 1`, `playable == False`. *Control 2:* one line with
  a 63-hex digest → `playable == False`. *Fails if:* a truncated or
  malformed sample counts as playable.
- F7 `steps_and_errors_order`. Canned record steps 4; transcript:
  `validate` exit 1 (`last line: unknown stem`), then `validate`
  exit 0 with one diagnostic message `missing ruleset`. Expects
  `steps == 4` and `errors` of length 2 in transcript order (the
  nonzero exit first). *Control:* transcript with both exits 0 and no
  diagnostics → `errors == []`, `steps` unchanged. *Fails if:*
  errors are counted but not ordered, or steps are recomputed.
- F8 `score_arithmetic`. (a) valid + playable + 0 errors / 4 steps →
  `100`. (b) valid + playable + 1 error / 4 steps →
  `50 + 30 + round(15.0) == 95`. (c) invalid + unplayable + 2 errors
  / 4 steps → `0 + 0 + round(10.0) == 10`. (d) 0 steps, 0 errors →
  divisor is 1, score is 100 when valid and playable. *Control:* (b)
  with 2 errors → `50 + 30 + round(10.0) == 90`, differing in one
  input from (b). *Fails if:* any of the four is off by one.
- F9 `compare_verdicts`. BASE brief `b2` score 80, CAND 95 → line
  `m-a b2 80 95 +15 better`, exit 0. *Control:* CAND 80 → `same`,
  exit 0. *Control 2:* CAND 60 → `worse`, exit 1. *Fails if:* a
  regression exits 0 or a tie reports `better`.
- F10 `no_subprocess_to_real_crpgc`. Runs the whole suite with `PATH`
  stripped of `crpgc` and no DNS (socket disabled by fixture):
  everything still passes because every external byte comes from
  `fake_crpgc.py` argv keys. *Control:* point the fixture at the
  real binary name and the test errors instead of silently passing.
  *Fails if:* any test shells to a real tool or the network.

Run command: `python -m unittest discover -s tools/tests -p
"test_author_bench.py" -v`. All ten pass; the pre-existing suites
(`test_oc.py` against `tools/tests/fake_opencode.py`,
`tools/agents/README.md:189`–`tools/agents/README.md:200`) are
untouched and still pass.

## What must not change
- Every existing suite and fixture, byte for byte: `tools/tests/`
  (notably `test_oc.py` over `tools/tests/fake_opencode.py`,
  `tools/agents/README.md:189`–`tools/agents/README.md:200`),
  `tools/lint/` and its self-tests (`AGENTS.md:73`–`AGENTS.md:75`),
  `crates/crpg-cli/tests/scaffolding.rs` (`:507`, `:530`, `:612`),
  `crates/crpg-cli/tests/inputs/llm-trials/` and `-r2/`.
- `schemas/` (23 checked-in files) and `crates/crpg-data/src/schema.rs`
  generated bytes; the seventeen `crpgc schema` stems
  (`crates/crpg-cli/src/main.rs:558`–`crates/crpg-cli/src/main.rs:579`).
- Protected paths (`tools/agents/verify_task.py:597`–`tools/agents/verify_task.py:603`):
  `Cargo.lock`, `deny.toml`, `Cargo.toml`, `rust-toolchain.toml`,
  `crates/crpg-contracts/` and `.github/workflows/`. No Rust file, no
  manifest, no workflow, no golden is touched: goldens none added, none
  moved.

## Dependency edges
No new edge. `tools/author_bench/*.py` are stdlib-only Python; the
Rust layering lint scans crate manifests only (`tools/lint/deps.py:1`–
`tools/lint/deps.py:37`, `ALLOWED` at `:46`), so there is no `ALLOWED`
row to add and `deps.py` is not edited. No external crate, no lockfile
package, no new dependency approval question (`AGENTS.md:52`):
the no-new-dependency alternative IS the design. Reused tools
(`tools/agents/oc.py`, `crpgc`, `schemas/`) are invoked, not imported.

## Out of scope
- Any change to `crpgc`, the schemas, the fixtures, the rulesets, or
  any Rust crate (pure consumer: `crpgc` invocations only, per the
  eleven subcommands of
  `crates/crpg-cli/src/main.rs:255`–`crates/crpg-cli/src/main.rs:271`).
- The T101 ADR itself (needs the T100 baseline,
  `docs/IMPLEMENTATION_PLAN.md:301`) and the T102 prototype.
- A bespoke model launcher or new agent role: `oc.py run --brief`
  (`tools/agents/README.md:122`–`tools/agents/README.md:126`) is reused
  as-is; no `oc.py`, `brief.py` or `verify_task.py` change.
- Human-judged quality (style, lore, "good design"): the bench scores
  only validity, playability, steps and errors.
- Re-blessing records: `--compare` never rewrites a baseline; a new
  baseline is a new scored run with its own `summary.json`.

## Open questions (each with a recommendation)

## Agent log

- 2026-11-19 (UTC) · muse-spark + T100 draft b · Skeleton.
