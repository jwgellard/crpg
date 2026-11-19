# T100 facts — agent authoring benchmark

Fact sheet gathered for T100 (agent authoring benchmark). Facts only; no
recommendations or proposed designs. Every `path:line` below was opened and
checked on this branch.

## Requirements

- T100 row (`docs/IMPLEMENTATION_PLAN.md:300`) — Kind `I`, owner OpenCode O1 from a manager contract; "Agent authoring benchmark under `tools/`: campaign briefs of rising size given to several models with only `crpgc` and the schemas; scores validity, playability (`crpgc run`), steps and errors. Baseline on today's tools first; re-run on each candidate command design. Includes a 'define a small rule system' brief beside the campaign briefs".
- T101 depends on T100 baseline (`docs/IMPLEMENTATION_PLAN.md:301`) — Authoring UX ADR; T100 baseline is a prerequisite.
- M6-R runs in parallel with M4 and needs no build slot (`docs/IMPLEMENTATION_PLAN.md:288`) — "### M6-R — Authoring UX research (D28j; runs now, in parallel with M4)".
- Wave 4 lists M6-R alongside (`docs/IMPLEMENTATION_PLAN.md:416`) — "M6-R (T099–T102) alongside, needing no build slot".
- M6-R agent log (`docs/IMPLEMENTATION_PLAN.md:459`) — added M6-R stage (T099–T103) that must close before any M6 contract.
- Spec status line (`docs/CRPG_ENGINE_SPEC.md:3`) — "Draft 1, for review. Written to be usable as the initial technical specification by a human architect and by AI coding agents."
- Spec audience (`docs/CRPG_ENGINE_SPEC.md:4`) — "solo developer with AI coding agents, plus the agents themselves."
- Spec line 419 (`docs/CRPG_ENGINE_SPEC.md:419`) — "Given that AI agents must generate and modify campaigns, and that campaigns must be diffable and collaborative, the ranking is:".
- Spec line 421 (`docs/CRPG_ENGINE_SPEC.md:421`) — "Machine-generatable by an LLM without training on a bespoke syntax."
- Spec lines 584–588 (`docs/CRPG_ENGINE_SPEC.md:584`–`docs/CRPG_ENGINE_SPEC.md:588`) — agent-facing `crpgc` commands: `schema`, `validate --json`, `new`, `explain`, dialogue text round-trip.
- Spec line 1013 (`docs/CRPG_ENGINE_SPEC.md:1013`) — "AI agents drive the same API headlessly. `crpgc apply commands.json` performs exactly what the GUI performs, including validation."
- Spec line 1569 (`docs/CRPG_ENGINE_SPEC.md:1569`) — "Definition of done: an LLM given only `crpgc schema creature` output produces a valid creature file on the first try. Test this literally."
- Spec line 2038 (`docs/CRPG_ENGINE_SPEC.md:2038`) — "make the campaign format usable by AI agents without the editor."
- Spec line 2042 (`docs/CRPG_ENGINE_SPEC.md:2042`) — "the literal acceptance test from Phase 2 — give an LLM only `crpgc schema creature` output and have it produce a file that passes `crpgc validate` on the first attempt. Record the transcript."
- Agentic workflow plan line 50 (`docs/AGENTIC_WORKFLOW_PLAN.md:50`) — "LLM consumption divides into five very different categories".
- Agentic workflow plan line 327 (`docs/AGENTIC_WORKFLOW_PLAN.md:327`) — "Keep the root instruction file small and stable."
- Agentic workflow plan line 330 (`docs/AGENTIC_WORKFLOW_PLAN.md:330`) — "Give the agent tools instead of files."
- D28j (`tasks/DECISIONS-2026-10-16.md:90`) — "authoring UX research stage (M6-R) before the editor contracts".
- D28j user quote (`tasks/DECISIONS-2026-10-16.md:92`) — "a major, major concern is both agent and human usability."
- D28j M6-R packages (`tasks/DECISIONS-2026-10-16.md:108`) — "The packages are T099–T103 in `docs/IMPLEMENTATION_PLAN.md` §3, M6-R."
- D28l audience (`tasks/DECISIONS-2026-10-16.md:125`) — "audience, rule-defining as a pillar, and both time modes".
- D28l T100 rule-system brief (`tasks/DECISIONS-2026-10-16.md:137`) — "T100 a rule-system brief".
- D28j agent log (`tasks/DECISIONS-2026-10-16.md:161`) — recorded user's same-day answers on the authoring UX research stage.
- D28l agent log (`tasks/DECISIONS-2026-10-16.md:163`) — recorded audience order, rule-defining pillar, both-time-modes.
- D29 draft answered (`tasks/DECISIONS-D29-DRAFT.md:34`) — "Answered 2026-10-16: module authors first, agents equal. Recorded as D28l".
- D29 draft D28j (`tasks/DECISIONS-D29-DRAFT.md:46`) — "agents stay a first-class audience (D28j)."

## The tools an authoring agent would get

Subcommand dispatch (`crates/crpg-cli/src/main.rs:255`–`crates/crpg-cli/src/main.rs:271`) recognises eleven subcommands: `replay`, `validate`, `migrate`, `new`, `schema`, `explain`, `fmt`, `lock`, `run`, `demo`, `play`.

- `replay` — usage `crates/crpg-cli/src/main.rs:32` (`REPLAY_USAGE`); combat replay entry `crates/crpg-cli/src/main.rs:2604` (`run_combat_replay`); legacy path calls `crpg_testkit::play_and_verify` at `crates/crpg-cli/src/main.rs:2631`. Reads a `.replay` file and a target-scoped golden; exits 0 on equality, 1 on any `ReplayError`, 2 on usage (`docs/architecture/crpg-cli.md:115`–`docs/architecture/crpg-cli.md:130`).
- `validate` — usage `crates/crpg-cli/src/main.rs:35` (`VALIDATE_USAGE`); entry `crates/crpg-cli/src/main.rs:1619` (`run_validate`). Collects campaign bytes, calls `crpg_data::validate_files`, renders diagnostics; exits 0/1/2 (`docs/architecture/crpg-cli.md:406`–`docs/architecture/crpg-cli.md:416`).
- `migrate` — usage `crates/crpg-cli/src/main.rs:39` (`MIGRATE_USAGE`); entry `crates/crpg-cli/src/main.rs:1738` (`run_migrate`). Explicit save over T012a migration-aware loader and canonical writer; silent on success; exits 0/1/2 (`docs/architecture/crpg-cli.md:500`–`docs/architecture/crpg-cli.md:510`).
- `new` — usage `crates/crpg-cli/src/main.rs:47` (`NEW_USAGE`); entry `crates/crpg-cli/src/main.rs:1775` (`run_new`). Scaffold types `creature`, `item`, `dialogue`, `quest` (`crates/crpg-cli/src/main.rs:165`–`crates/crpg-cli/src/main.rs:171`); writes one canonical document to stdout.
- `schema` — usage `crates/crpg-cli/src/main.rs:48` (`SCHEMA_USAGE`); entry `crates/crpg-cli/src/main.rs:1845` (`run_schema`). Seventeen stems listed in `is_schema_stem` (`crates/crpg-cli/src/main.rs:558`–`crates/crpg-cli/src/main.rs:579`); writes generated schema bytes.
- `explain` — usage `crates/crpg-cli/src/main.rs:49` (`EXPLAIN_USAGE`); entry `crates/crpg-cli/src/main.rs:1918` (`run_explain`). Calls `explain_object` on a loaded campaign; prints `Some(bytes)` or `object not found`.
- `fmt` — usage `crates/crpg-cli/src/main.rs:50` (`FMT_USAGE`); entry `crates/crpg-cli/src/main.rs:2025` (`run_fmt`). Check/save canonical formatting; `--check` prints `crpgc fmt: noncanonical: <logical>`.
- `lock` — usage `crates/crpg-cli/src/main.rs:51` (`LOCK_USAGE`); entry `crates/crpg-cli/src/main.rs:2493` (`run_lock`). Reads `campaign.json`, `assets/assets.lock`, and a flat catalog; writes `campaign.lock`.
- `run` — usage `crates/crpg-cli/src/main.rs:52` (`RUN_USAGE`); entry `crates/crpg-cli/src/main.rs:2518` (`run_run`). Calls `crpg_testkit::run_hash_sequence` once with an empty no-op script (`crates/crpg-cli/src/main.rs:2519`); prints samples `<k> <64-hex>` for completed ticks M, 2M, … ≤ N.
- `demo` — usage `crates/crpg-cli/src/main.rs:54` (`DEMO_USAGE`); entry `crates/crpg-cli/src/demo.rs:44` (`demo::run`). Terminal combat viewer over testkit playback and the private `combat_apply` adapter.
- `play` — usage `crates/crpg-cli/src/main.rs:58` (`PLAY_USAGE`); entry `crates/crpg-cli/src/play.rs:47` (`play::run`). Playable terminal combat choosing from `crpg_sim::legal_actions`.

Published schemas: `crates/crpg-data/src/schema.rs:94` (`generated_schemas`) returns 23 filename-to-byte schemas; stems listed at `crates/crpg-cli/src/main.rs:558`–`crates/crpg-cli/src/main.rs:579` (17 `crpgc schema` stems); checked-in files under `schemas/` (23 files).

## What "playable" can be checked by today

- `crpgc run` (`crates/crpg-cli/src/main.rs:2518`) is a bounded hash harness over an empty world, not campaign execution; it prints sampled state hashes only (`docs/architecture/crpg-cli.md:766`–`docs/architecture/crpg-cli.md:768`).
- `crpgc demo` (`crates/crpg-cli/src/demo.rs:44`) plays a combat replay through testkit (`read_replay` + `play_replay`) and the private `combat_apply` adapter, printing header, per-intent blocks, round/turn/death lines, and a summary (`docs/architecture/crpg-cli.md:294`–`docs/architecture/crpg-cli.md:340`).
- `crpgc play` (`crates/crpg-cli/src/play.rs:47`) loads a campaign, binds an encounter via the adapter's `start`, then loops over the active combatant offering `crpg_sim::legal_actions` and performing choices through `crpg_sim::perform_action` (`docs/architecture/crpg-cli.md:341`–`docs/architecture/crpg-cli.md:390`).
- Campaign load and validation for playability: `crpgc validate` (`crates/crpg-cli/src/main.rs:1619`) runs `crpg_data::validate_files` over collected campaign bytes; combat replay requires an empty findings list (`docs/architecture/crpg-cli.md:170`–`docs/architecture/crpg-cli.md:182`).
- Headless combat proof in testkit: `tests/combat_replay.rs` and `tests/support/combat.rs` load `campaigns/fixtures/combat_basic/` through production `load_campaign`, bind placement ULIDs to entities, decode `{"combat":1,"op":"init"|"attack"|"end"}`, and run through `start_encounter`/`perform_action` (`crates/crpg-testkit/AGENTS.md` "Combat replay (T016c)" section); `tests/srd_replay.rs` does the same for `combat_srd` (`crates/crpg-testkit/AGENTS.md` "Second-ruleset replay (T017e)" section).
- Replay verification primitives: `crpg_testkit::play_and_verify` (`crates/crpg-testkit/src/replay.rs:442`), `play_replay` (`crates/crpg-testkit/src/replay.rs:371`), `read_replay` (`crates/crpg-testkit/src/replay.rs:424`).
- Hash harness primitive: `crpg_testkit::run_hash_sequence` (`crates/crpg-testkit/src/harness.rs:186`).

## Existing campaigns and rulesets to size briefs against

- `campaigns/fixtures/combat_basic/` — 15 files, 262 total lines; minimal d6 combat campaign (hero/goblin, strike ability, first-blood encounter, minimal-d6 ruleset).
- `campaigns/fixtures/combat_srd/` — 18 files, 408 total lines; srd-lite combat campaign (hero/goblin, focus/heavy abilities, effects, two outcome tables, srd-lite ruleset).
- `rulesets/minimal-d6/` — 4 files, 89 total lines; minimal d6 ruleset package (ruleset, strike ability, roll_under outcome table).
- `rulesets/srd-lite/` — 7 files, 230 total lines; srd-lite ruleset package (ruleset, focus/heavy abilities, focusing effect, two outcome tables).
- `crates/crpg-data/tests/fixtures/migration_v1/campaign/` — 14 files, 210 total lines; migration fixture campaign used by migrate golden tests.

## Existing agent-running scripts to reuse

- `tools/agents/oc.py` — dispatcher; starts OpenCode workers in warm git worktree slots, runs the verifier, prints verdict-first summaries; `run --task ID --role ROLE --model MODEL` prepares a slot and branch (`tools/agents/README.md:9`–`tools/agents/README.md:28`).
- Slot auto-pick and `slot-occupied` guard (`tools/agents/README.md:33`–`tools/agents/README.md:41`).
- Result records under `<repo>/.agents/runs/<ID>/` (`tools/agents/README.md:41`–`tools/agents/README.md:49`).
- `record` subcommand (`tools/agents/oc.py:2404`, `do_record`) appends an outcome entry to a result record; `REVIEW_OUTCOMES` (`tools/agents/oc.py:37`) = `review-ready`, `review-fix`, `review-reject`, `merged`, `reverted`; blank record shape in `new_info` (`tools/agents/oc.py:1747`) includes `task`, `role`, `model`, `attempt`, `started_utc`, `elapsed_s`, `verdict`, `cause`, `failed_checks`, `steps`, `tokens`, `cost`, `retries`, `outcomes`, `last_text`.
- `--json` record output: `run`/`wave`/`ask` print one JSON object with `tool`, `version`, `members`/result fields (`tools/agents/README.md:6`).
- `tools/agents/README.md` documents the scripts and the doc-only slot rule (`tools/agents/README.md:233`–`tools/agents/README.md:236`).
- Literal LLM acceptance test: `crates/crpg-cli/tests/scaffolding.rs:502` defines `LLM_TRIALS` and `crates/crpg-cli/tests/scaffolding.rs:530` defines `LLM_TRIALS_R2`; test `crates/crpg-cli/tests/scaffolding.rs:612` `llm_trial_responses_validate_in_private_campaigns` installs preserved responses into private campaigns and requires `validate --json` success (`docs/architecture/crpg-cli.md:792`–`docs/architecture/crpg-cli.md:797`).
- Preserved trial inputs: `crates/crpg-cli/tests/inputs/llm-trials/` (creature, item, dialogue, quest) and `crates/crpg-cli/tests/inputs/llm-trials-r2/`.

## Tool rules that bind it

- Tools layout (`tools/README.md:1`–`tools/README.md:9`) — `lint/`, `preflight.sh`/`preflight.ps1`, `tests/`; nothing here is a Rust crate or CI workflow.
- Lint self-test rule (`AGENTS.md:73`–`AGENTS.md:75`) — CI runs `python tools/lint/deps.py`, `python tools/lint/determinism.py`, and `python -m unittest discover -s tools/lint -p "test_*.py"`; `--all-targets` matters.
- Dependency lint (`tools/lint/deps.py`) and determinism lint (`tools/lint/determinism.py`) are the enforced non-negotiables (`AGENTS.md:28`–`AGENTS.md:48`).
- Protected files (`tools/agents/verify_task.py:596`–`tools/agents/verify_task.py:603`) — `Cargo.lock`, `deny.toml`, `Cargo.toml`, `rust-toolchain.toml`, and `crates/crpg-contracts` (and everything under it), and `.github/workflows` (and everything under it).
- Doc-only slots (`tools/agents/README.md:233`–`tools/agents/README.md:236`) — slots 3–6 and any slot whose brief `Route:` contains `doc-only` never run cargo; with a crate owned they report `skipped (doc-only slot)`, else they run tool unittests when `tools/`, `.opencode/` or `opencode.json` changed.
- Network/model-use from scripts: `tools/agents/brief.py:8` states "Stdlib only; no network."; `tools/agents/recall.py:7` states "only; no network."; test suites run without the network or real OpenCode (`tools/agents/README.md:196`).
- No new dependencies without approval (`AGENTS.md:52`); no `unsafe` outside `crpg-godot` (`AGENTS.md:25`–`AGENTS.md:27`); no OS-specific branches in core/rules/sim (`AGENTS.md:17`–`AGENTS.md:19`).

## Agent log

- 2026-11-19 (UTC) · opencode-go/beam-free + T100 facts · Gathered facts with path:line citations for the agent authoring benchmark task so a contract can be written from them.