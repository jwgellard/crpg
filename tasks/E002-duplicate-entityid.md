## Task
Reconcile the duplicate `EntityId` definition: spec §2.4 / Simulator still
defines a separate `EntityId` in `crpg-sim`, contradicting ADR-0006 which put
the typed arena and `EntityId` in `crpg-core`. Human-decision task.

## Why this is deferred

ADR-0006 (arena in core) is already implemented and merged into the uncommitted
work. The spec's Simulator section (§2.4) still describes a parallel `EntityId`
in `crpg-sim`. If T007 builds `crpg-sim` from that spec section it will
re-introduce a second, incompatible id type. This needs a spec edit, which is a
product/design decision the maintainer must sign off.

## Decision to make
- Confirm ADR-0006 stands (single `EntityId` in core).
- Edit the spec §2.4 Simulator section to reference the core `EntityId`
  (via ADR-0006) instead of redefining its own.

## Deliverable
- Update `docs/CRPG_ENGINE_SPEC.md` §2.4 to remove the duplicate `EntityId`
  definition and point at ADR-0006.
- Cross-check `tasks/T007.md` (when written) that it uses the core id.

## Constraints
- Do not touch `crpg-*` sources in this task; only the spec and any ADR note.
- Human-decision: do not guess; get sign-off before editing the spec.

## Agent log

- 2026-09-05 (UTC) · opencode/muse-spark · Applied per maintainer sign-off: spec §2.4 now references the single core EntityId; see spec Agent log.
