# crpg-rules architecture

## Scope

`crpg-rules` holds the game-system-neutral stat and modifier kernel from
spec §24 T14 plus the T015 dice, outcome-table, resolution, and resource
surface. The implementation contracts are [T014](../../tasks/T014.md) and
[T015](../../tasks/T015.md); verification status belongs to the task
records. The only internal dependency is `crpg-core`. There is no filesystem
access, no JSON ruleset loader, no campaign schema, and no `crpg-data`
dependency: rules owns typed in-memory definitions, and a future data task
supplies the authoring-format adapter. There is no entity registry, effect
store, scheduler, hook dispatcher, or damage engine here either.

## Module flow

- `stats` holds the staged value model (`Int`, `Fixed`, `Bool`, `Enum`,
  `Tags`, and the T015 `Dice` expression values), `StatKind` declarations
  (`Dice` included), `Expr` trees, `StatDefinition`, the
  insertion-ordered `StatBlock` with its standalone validation, and the
  serde-only symbolic DTOs with the explicit string-conversion boundary in
  both directions (failed loads roll back before any interner mutation;
  dice values travel as canonical notation parsed in preflight).
- `derived` holds dependency-graph validation (reference resolution, cycle
  detection with canonical rotated paths, acyclic-depth bound) and the
  fully-modified expression evaluator shared by queries. Dice values pass
  through literals and references only; binary operators still require
  matching `Int` or `Fixed` operands.
- `modifier` holds modifier identities (`Ulid`), `SourceRef`, `ModOp`,
  `ConditionExpr`, data-selected `StackingPolicy`, the immutable validated
  `ModifierPipeline`, pure borrowed `QueryContext` queries, and the
  always-present `ModifierBreakdown` trace DAG. Modifier targets cover stats
  plus roll/DC tags (`RollTag` wraps a caller-issued `TagId`); the integer
  `query_numeric` fold reuses the same selection/folding helpers as the stat
  path rather than reimplementing stacking.
- `dice` holds the bounded single-group expression grammar (`DiceExpr` with
  its parser, canonical display, and string-form serde), the keep/drop
  selection over authored draw order, and evaluation against a
  caller-selected named stream of the caller's `DeterministicRng`.
- `resolution` holds the validated data-driven outcome tables (margin bands
  with implicit exclusive upper bounds, ordered natural-face rules that
  shift the selected band index or override its outcome) and the generic
  `resolve` entry point: validated existence/shape/context checks, union-tag
  roll conditions, preflight DC/stat evaluation whose traces are reused,
  staged-clone transactional dice draws, and margin selection with the
  actor's raw natural face.
- `resource` holds the caller-clocked `ResourcePool` state (spend with
  insufficient-funds rollback, idempotent refresh against host-delivered
  turn/round/rest/tick events) with constructor-validated serde.
- `hooks` holds the nine-variant `KernelHook` vocabulary with its serde wire
  shapes, the runtime-only `HookMutation` proposal type, and the pure
  `HookHandler` signature. There is no registry or dispatcher here. The T015
  roll/damage variants carry correlation ULIDs plus core-closed entity and
  amount fields; `resolve` emits nothing.
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
borrowed context entity only. Authored table, pool, and rest identities are
ULIDs; runtime roll identifiers wrap existing tag handles whose issuing
interner is the caller's responsibility. Hook payloads carry core-closed fields only, per
[ADR-0008](../adr/0008-event-ownership.md) and
[ADR-0011](../adr/0011-event-payload-fields.md); the hook vocabulary lives
here while dispatch lives above. The host supplies resolution correlation
ULIDs and owns hook emission points; damage-hook amounts are
caller-computed, caller-applied, nonnegative values that rules neither
calculates nor applies. Effect, action, and movement hooks wait for their
owning capabilities.

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

## Dice, resolution, and resources

A dice expression is one dice group plus an optional integer offset; the
grammar accepts no whitespace, omitted counts, or general arithmetic.
Evaluation draws every die in authored index order from the caller's named
stream (one-sided dice still consume a range draw), ranks keep/drop
selection with earlier indices winning value ties, keeps the trace in draw
order, and clamps the kept sum plus offset to `i32` once. Querying a
dice-valued stat returns the expression; only explicit evaluation or
resolution consumes randomness.

Resolution validates table identity, entity existence, roll shapes, and each
unique participant context once in ascending entity order, checks the
actor/opponent tag unions against the tag bound, evaluates stat-based DCs in
preflight and reuses those traces, draws actor then opponent dice on a staged
RNG clone in that order, folds roll modifiers onto each raw total (constant
and stat DCs receive their owner's DC modifiers; opposed defence is the
opponent's final total), and commits the staged RNG only after complete
success. The margin is an unsaturated `i64` difference and the table decides
zero-margin ties; natural-face rules inspect the actor's raw selected face.

Resource pools are plain caller-clocked counters: spend fails without
mutation on insufficient funds, refresh refills to max only on matching
host-delivered events (tick `n` matches positive ticks divisible by `n`,
tick zero never refreshes) and reports whether anything changed.

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
- 2026-09-26 (UTC) · opencode/muse-spark + T015 crate opening · Extended scope, module flow, authorities, and arithmetic sections with the dice, resolution, and resource surface before source implementation; T014 boundaries and ownership rules preserved.
