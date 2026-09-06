## Task
Specify the shapes T018 must encode before the protocol task is written.
Human-decision task (spec edit + task scoping).

## Why this is deferred

T018 owns `ClientIntent`/`DeltaOp` enums, the `postcard` codec with version
byte, and conformance/desync tests (`docs/CRPG_ENGINE_SPEC.md:1705-1711`,
`tasks/BACKLOG.md:86`), but every shape it encodes is a sketch or missing:

- `SimEvent` variants: one example (`Damage {target,amount,type,outcome,
  source}`, spec `:750`) and a `SimEvent(SimEvent)` wrapper (`:669`) — no
  variant list, versioning, or interaction with per-client filtering (`:695`).
- `ClientIntent` fields (`:680-686`): position/tick/ability/target/net-id
  types missing; spec uses `tick` while the ADR-0004 spike used per-intent
  `seq` (`docs/adr/0004-quic-movement-spike.md:19-24`); no validation
  taxonomy or idempotency rule (`tasks/T002.md:61` already flags the match).
- Action registry: IR `Action(action_id,args)` lives in `crpg-data` per
  ADR-0008, which cannot depend on rules/sim (`tools/lint/deps.py:42-44`) —
  the indirection from IR reference to Rust implementation is undescribed,
  and no owner holds the signature store (`:532,903,925`, spec `:161`).
- Intent rate limits + payload caps (`:689`) have no values, scope, or
  rejection codes, though the malicious-client suite (`:1176`) must test them.
- `legal_actions -> Vec<ActionOption>` (`:620`), `visible_to`
  (`:695`), `InterestSet` (`:694`), `ScriptContinuation` (`:527`) are named
  with undefined types; interest/filtering/prediction/reconnect beyond the
  in-memory transport have no follow-on task (T018 covers codec/in-memory/
  desync only).

## Decision to make
- Fix the `seq`-vs-`tick` discrepancy; type intents, deltas, and the action
  indirection; set caps and rejection codes; scope T018 vs named follow-ons
  (interest impl, prediction, grace-window behavior).

## Deliverable
- Edits to `docs/CRPG_ENGINE_SPEC.md` (§§5.2, 6.2, 7.3–7.6) with dated notes
  + BACKLOG T018 scope + follow-on rows. No source changes.

## Constraints
- Doc-only. Human sign-off; T018 is unwritable against sketches.

## Agent log

- 2026-09-06 (UTC) · opencode/muse-spark · Filed as part of the spec-gap triage: everything T018 encodes is currently a sketch.
