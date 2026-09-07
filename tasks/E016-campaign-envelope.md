## Task
Resolve the campaign-format contradictions before the T010 schema is written.
**Decided 2026-09-06: explicit document and identity domains.**

## Original problem

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

## Decision
- Entity documents contain one independently identified object. Named
  aggregate documents contain a parent-owned collection or map; the initial
  exceptions are placements, triggers, locale tables, and campaign variables.
- `assets.lock` is authoritative for source asset hashes and import settings.
  `campaign.lock` owns resolved package versions/checksums and one digest of
  `assets.lock`. The package manifest owns final packaged-content hashes.
- Authored-object references use ULIDs. Dependency `requires[]` entries use a
  separate immutable, human-readable package id in a field named `package`.
- Event-IR `Wait` stores a relative `u64` simulation-tick count.
- T010 owns schemas, the object index, deterministic resolution, and lockfile
  read/write APIs. T012 remains migration-only. T013 exposes `crpgc lock` as a
  thin caller of T010 rather than duplicating data behavior in the CLI.

## Consequences
- Every JSON file still contains exactly one canonical JSON value, without
  forcing high-churn collections into artificial per-entry files.
- No two lockfiles independently claim authority over the same asset entries.
- Package resolution remains readable while object references retain stable
  globally unique identity.
- T010 can define the IR without importing rules-specific rounds or turns.

## Constraints
- Doc-only. T010 inherits these format decisions; changing an identity or hash
  authority after implementation requires an ADR and migration plan.

## Agent log

- 2026-09-06 (UTC) · opencode/muse-spark · Filed as part of the spec-gap triage: four format contradictions, all owned by nobody, all due at T010.
- 2026-09-06 (UTC) · opencode/gpt-5.6-sol + maintainer decision · Chose explicit aggregate documents, separate lock authorities, distinct package ids, and tick waits; assigned data behavior to T010 and only its CLI surface to T013.
