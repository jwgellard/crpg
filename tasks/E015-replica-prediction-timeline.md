## Task
Define the replica/prediction model and own the `Timeline`. Human-decision
task (spec edit + task assignment).

## Why this is deferred

- The client does "prediction of own movement only"
  (`docs/CRPG_ENGINE_SPEC.md:23-24`) — prediction needs speculative local
  mutation — but the client's copy sits "behind a `ReplicaWorld` type with no
  mutating methods except `apply_delta`" (`:135-138`). The two are never
  reconciled, and `apply_delta` has no shape.
- The `Timeline` (ordered `(initiative_key, EntityId)` queue unifying
  real-time and turn-based, `:253`) has no key type, no tiebreak, no advance
  rule, and no owner; §10 ticks AI every tick (`:796-805`) while §6.2 scores
  combat AI "on turn start" (`:621`) with no turn-vs-tick mapping.
- `ComponentStore<T>`, `DynamicComponentStore` ("typed by schema" — which
  schema?), `SpatialIndex` (cell size? rebuild cost?), `EntityMeta`,
  `Transform` are named in the `World` sketch (`:222-236`) but shaped nowhere;
  T007 (`tasks/BACKLOG.md:47`) owns `ComponentStore` + spawn/despawn/query
  only.

## Decision to make
- Choose the replica mutation story (prediction buffer vs mutable replica
  with rollback discipline); specify `apply_delta` coverage and the
  subscription/interest hookup; type the `Timeline` and assign it, the
  spatial index, and the dynamic store to T007/T008 explicitly.

## Deliverable
- Edits to `docs/CRPG_ENGINE_SPEC.md` (§§2.1, 2.4, 2.5, 10) with dated notes
  + BACKLOG T007/T008 scope lines. No source changes.

## Constraints
- Doc-only. Human sign-off; T007 locks `World` shape and must reserve the
  replica/event/interest seams (cf. E009).

## Agent log

- 2026-09-06 (UTC) · opencode/muse-spark · Filed as part of the spec-gap triage: prediction-vs-immutable-replica plus four unowned World members.
- 2026-09-06 (UTC) · opencode/muse-spark + maintainer sign-off · Replica stays immutable with the prediction buffer outside sim; `Timeline` is `BTreeMap<(InitiativeKey(i32), EntityId)>`, container in T007, advance rules (incl. §6.2/§10 mapping) in T008; spatial index and dynamic store deferred with reservations; spec §§2.1/2.4/2.5/24 and BACKLOG T007/T008 updated.
