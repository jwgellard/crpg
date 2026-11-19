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

## Behaviour

## Errors

## Tests

## What must not change

## Dependency edges

## Out of scope

## Open questions (each with a recommendation)

## Agent log

- 2026-11-19 (UTC) · muse-spark + T100 draft b · Skeleton.
