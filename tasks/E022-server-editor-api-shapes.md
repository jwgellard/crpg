## Task
Ledger the unspecified server/editor/bridge API shapes so post-T018 tracks
have a design queue. Human-decision task (scoping ledger, no design yet).

## Why this is deferred

BACKLOG ends at T018, but §§9–11 name APIs with verbs and no signatures:

- FFI surface (§11.3: five object types, verb lists, zero signatures, error
  model, threading/ownership, versioning).
- Scene-sync / replica / query protocol (`world.stat(entity,"hp")`,
  `entities_in_area()`, interpolation buffer, `net_id`↔`EntityId` mapping,
  stringly `"hp"` vs interned `StatId`).
- `EditCommand` variants (`{..}` placeholders), receipts, diagnostics,
  identity (`JsonPointer` vs ULID), `crpgc apply` (not in any CLI inventory),
  live-edit conflict semantics.
- Handshake/auth/session/rate-limit policy (accounts deferred per §22, so
  what *is* auth?), validation-failure policy, asset-policy verification
  flow, signing trust root and offline behavior.

## Decision to make
- No API design in this task — only the ledger: per-shape owning crate,
  prerequisite task, and phase. (The capability *model* for the privileged
  channel is E018; this covers the *mechanisms*. Auth policy details may
  merge into E018 at sign-off.)

## Deliverable
- BACKLOG "not yet numbered" extension rows with owners + a dated note in
  spec §§9–11 pointing at the ledger. No source changes.

## Constraints
- Scoping-only. Human sign-off; prevents post-T018 tracks from designing
  against verbs.

## Agent log

- 2026-09-06 (UTC) · opencode/muse-spark · Filed as part of the spec-gap triage: verbs without signatures, inventoried before anyone builds on them.
