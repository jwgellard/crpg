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

## Readings

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
