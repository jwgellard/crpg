# crpg-rules architecture

## Scope

T014 opens `crpg-rules` with the game-system-neutral stat and modifier kernel
from spec §24 T14. The implementation contract is
[T014](../../tasks/T014.md); verification status belongs to the task record.
The only internal dependency is `crpg-core`. There is no filesystem access, no
JSON ruleset loader, no campaign schema, and no `crpg-data` dependency: rules
owns typed in-memory definitions, and a future data task supplies the
authoring-format adapter.

## Module flow

- `stats` holds the staged value model (`Int`, `Fixed`, `Bool`, `Enum`,
  `Tags`), `StatKind` declarations, `Expr` trees, `StatDefinition`, the
  insertion-ordered `StatBlock` with its standalone validation, and the
  serde-only symbolic DTOs with the explicit string-conversion boundary in
  both directions (failed loads roll back before any interner mutation).
- `derived` holds dependency-graph validation (reference resolution, cycle
  detection with canonical rotated paths, acyclic-depth bound) and the
  fully-modified expression evaluator shared by queries.
- `modifier` holds modifier identities (`Ulid`), `SourceRef`, `ModOp`,
  `ConditionExpr`, data-selected `StackingPolicy`, the immutable validated
  `ModifierPipeline`, pure borrowed `QueryContext` queries, and the
  always-present `ModifierBreakdown` trace DAG.
- `hooks` holds the five-variant `KernelHook` vocabulary with its serde wire
  shapes, the runtime-only `HookMutation` proposal type, and the pure
  `HookHandler` signature. There is no registry or dispatcher here.
- `error` exposes the owned structured `RulesError`/`RulesErrorCode` model
  with deterministic diagnostic paths.

A query borrows one entity's base block, tag set, and caller-owned modifier
slice; the pipeline validates the context in a fixed phase order, evaluates
the requested dependency closure with per-query memoization, folds retained
modifiers in canonical phase/priority/ID order with saturating floor
arithmetic, and returns the value with a complete breakdown. Nothing is cached
between queries and base state is never mutated: removing a source means
filtering its modifiers out and querying again.

## Authorities and consumers

Stat and tag strings are the persisted authority; `StatId`/`TagId` handles
are runtime-only per [ADR-0006](../adr/0006-crpg-core-primitives.md)
Decision 4 and are never serialized. Modifier membership, entity liveness,
and authority belong to the host caller; rules checks equality against the
borrowed context entity only. Hook payloads carry core-closed fields only, per
[ADR-0008](../adr/0008-event-ownership.md) and
[ADR-0011](../adr/0011-event-payload-fields.md); the hook vocabulary lives
here while dispatch lives above. Dice-valued stats, `DiceExpr`, roll/DC
modifier targets, and resolution-dependent hooks are explicitly staged to
T015; effect, action, and movement hooks wait for their owning capabilities.

## Canonical arithmetic and trace DAG

Int expression and modifier arithmetic uses `i64` intermediates with
saturation to `i32` after each operation; Int division floors toward negative
infinity with `MIN / -1` saturating and division by zero rejected before core
division runs. Fixed arithmetic uses the existing saturating/floor `Fx16_16`
methods. `ModOp::Multiply` on Int uses the full-range raw scaling formula so
large integers never pass through the fixed-point range. Derived references
read fully modified values on the same entity; conditions read only supplied
tags. The flat traces map holds each transitively evaluated stat exactly once
(diamonds do not duplicate subtrees) with sorted direct-dependency lists, and
every contribution — applied, condition-false, zero, or suppressed — is
ordered by phase then ascending `(priority, id)`.

## Governing decisions

Spec §3 (values), §15 (workflow), §24 T14/T15 (kernel scope and dice
staging); [ADR-0006](../adr/0006-crpg-core-primitives.md) Decisions 2 and 4
(fixed-point semantics, string persistence boundary);
[ADR-0008](../adr/0008-event-ownership.md) and
[ADR-0011](../adr/0011-event-payload-fields.md) (hook ownership and
payload-field bound);
[ADR-0012](../adr/0012-windows-primary-platform.md) (native gates, no
OS-specific branches in this crate).

## Agent log

- 2026-09-26 (UTC) · opencode/muse-spark + T014 crate opening · Created this document before source implementation to satisfy the §15.6 readiness gate, describing the approved T014 kernel boundaries and module flow rather than planned work.
- 2026-09-26 (UTC) · opencode/muse-spark + T014 implementation alignment · Corrected the module flow: symbolic DTOs and conversions live in `stats`, not a separate module, matching the delivered code; no boundary change.
