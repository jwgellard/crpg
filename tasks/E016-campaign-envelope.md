## Task
Resolve the campaign-format contradictions before the T010 schema is written.
Human-decision task (spec edit).

## Why this is deferred

- "One document per file, JSON, one object per file"
  (`docs/CRPG_ENGINE_SPEC.md:363`) vs mandated aggregate files —
  `placements.json`, `triggers.json`, locale tables,
  `variables/campaign_state.json` (`:398-427`). No exemption rule stated.
- Two lockfiles with overlapping asset-hash duties: `campaign.lock`
  ("resolved dependency versions + asset hashes," `:401`) vs
  `assets/assets.lock` ("path → blake3 hash," `:422,434`). No authority rule,
  no sync story.
- "`id` is a ULID … All cross-references use `id`" (`:391`) vs `requires[]`
  using slugs (`pf2e`, `core-assets`, `:455-457`). No rule for which id
  domain `requires` lives in.
- `Wait(duration)` (`:520`) has no unit against §2.5's rounds/turns/ticks,
  never seconds (`:252`). The IR schema cannot be written without choosing.

## Decision to make
- State the aggregate-file exemption; pick the lockfile authority (or merge
  them); define the `requires[]` id domain; fix `Wait`'s unit. Assign the
  resolver, lock writer, and loader index (`id→(type,path)`) to T010/T012.

## Deliverable
- Edits to `docs/CRPG_ENGINE_SPEC.md` (§§4.1–4.5, 5.2) with dated notes +
  BACKLOG T010/T011/T012 scope lines. No source changes.

## Constraints
- Doc-only. Human sign-off; T010 is the schema task and inherits all four
  answers.

## Agent log

- 2026-09-06 (UTC) · opencode/muse-spark · Filed as part of the spec-gap triage: four format contradictions, all owned by nobody, all due at T010.
