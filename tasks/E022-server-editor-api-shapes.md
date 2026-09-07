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

## T009c Host Ledger

Status remains open. Under ADR-0012/T009c, ledger the shared authoritative host
API/package decision with E012 before the server hosting implementation tasks.
The required implementation is one platform-neutral authority reused by
Windows embedded single-player, Windows dedicated, and Linux dedicated
adapters. Do not choose here whether it is a library target in `crpg-server`
or code in another existing crate; E012/E022 own that decision and must assign
the eventual owning crate, prerequisites, and phase before implementation.

Ledger transport/lifecycle, ownership/threading, shutdown, and error boundaries
for those adapters without inventing signatures. In-process transport is still
the client/server boundary: clients never mutate authoritative state directly.
OS-specific hosting concerns stay above core/rules/sim. Windows/MSVC is primary;
Linux/GNU headless server, CLI/tooling, server-side extensibility, and testing
are fully supported and must not depend on Godot. Linux GUI client/editor
builds are not promised.

Future owning tasks must activate real Windows embedded-server and dedicated-
server smoke tests plus a Linux dedicated-server smoke test when available,
alongside real Windows client/editor/server and Linux headless server build
artifacts (E020: no placeholder jobs). E023 separately owns native-extension
ABI/loading, target artifacts, packaging, and reconciliation with the rule
that only `crpg-godot` may use `unsafe`; no implementation is selected here.

## Agent log

- 2026-09-06 (UTC) · opencode/muse-spark · Filed as part of the spec-gap triage: verbs without signatures, inventoried before anyone builds on them.
- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c · Added shared-host API/package scoping for Windows embedded/dedicated and Linux dedicated adapters without choosing placement or signatures. Recorded authority boundaries, capability-gated smoke/build obligations, and the separate E023 native-extension governance owner.
