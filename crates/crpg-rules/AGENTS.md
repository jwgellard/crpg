# crpg-rules — agent contract

Read the root rules and [T014](../../tasks/T014.md), the binding kernel API
and acceptance contract. Architecture: [crpg-rules](../../docs/architecture/crpg-rules.md).
This document describes the T014 stat/modifier kernel; results live in T014.

## Public surface

Public modules `stats`, `modifier`, `derived`, `hooks`, and `error`
re-export their public items at the root. T014 lists every required type,
field, variant, function signature, and limit constant. Operations are
`StatBlock::{new, insert, get, remove, iter, len, is_empty, to_serializable,
from_serializable}`, `ModifierPipeline::{new, query}`, and the pure
`HookHandler` call convention. Do not extend the surface: no new trait, no
mutable query cache, no subtract-to-undo method, no handler registry or
dispatcher, no production JSON writer, no second public abstraction.

## Mutation and query rules

- `StatBlock` is the only mutable state. `insert` validates standalone
  shape/limits before changing anything; invalid inserts leave it unchanged.
  Equality is by key/value contents, never insertion order. There is no raw
  map exposure, mutable getter, serde impl, or alternate mutation path.
- Queries are pure borrows over exactly one entity. The caller owns modifier
  membership and lifetime: removal is filter-by-`SourceRef` plus re-query
  from base state, never arithmetic inversion. No cache survives a query.
- Validation order is contract: `new` checks limits, duplicate IDs,
  declaration/domain/policy shape, references, cycles/depth, then expression
  types; `query` checks entity equality, stat existence, context limits,
  stored entries by ascending `StatId`, modifier ID uniqueness, modifiers by
  ascending ID, then the dependency closure. Multi-fault inputs report the
  first failure in that order; error paths mutate nothing.
- Stacking is data-selected per `mod_type`: `StackAll` keeps all candidates,
  `HighestBonusWorstPenalty` keeps the largest positive and smallest negative
  `Add` independently (zeros are `ZeroAdd`), `HighestPriorityPerName` keeps
  the greatest `(priority, id)` per exact name within each phase group.
  Application is global phase order `Set -> Add -> Multiply -> Clamp`, then
  ascending `(priority, id)`. Higher priority runs later; greater ID wins
  ties in both selection and application.
- Every successful query returns a complete `ModifierBreakdown`: root trace
  present, returned value equals its value, flat per-stat traces with sorted
  dependencies, all contributions ordered by phase then `(priority, id)`.
- Persistence converts through the caller's issuing `Interners` only.
  `from_serializable` validates everything before interning anything, interns
  in canonical lexical order, and leaves the interner unchanged on error.
  Never serialize numeric handles; never add serde to core handle types.

## Determinism traps

- No `HashMap`/`HashSet` anywhere, including tests and doctests. No
  `f32`/`f64`, no unsuffixed float literals, no wall clock, no threads, no
  external RNG. The lint scans doctest fences too.
- Int arithmetic saturates per operation via `i64` intermediates; never
  regroup additions or multiplications. Int division floors (adjust for
  negative remainders); never use `div_euclid` with a possibly negative
  divisor. Integer modifier scaling uses the raw `to_raw` formula, never
  `Fx16_16::from_int`, or large values clamp at 32767.
- Fixed arithmetic uses core's saturating/floor methods only. Check for zero
  divisors before calling core division: core saturates, rules must report
  `DivisionByZero`.
- Check expression/condition tree depth and node counts iteratively before
  recursing. Traverse definition roots and reference neighbours in ascending
  `StatId` order; rotate reported cycles to the smallest `StatId`.
- Cycle `Display` stays `<Code> at <location>`; cycle members are structural
  (`cycle` field) and runtime-index paths are never persisted identities.
- The determinism lint reads tuple field access after `]` or `)` as a float
  literal (`entries[0].0` trips `no-float`): destructure instead
  (`let (first, _) = entries[0];`).
- Test stat/type names stay neutral: no attribute, armour, health, class,
  level, or die assumptions in production or tests. Dice and roll/DC hooks
  are T015; do not anticipate them here.

## Limits

`MAX_STATS` 1024, `MAX_TAGS` 1024, `MAX_ENUM_VARIANTS` 1024, `MAX_POLICIES`
1024, `MAX_MODIFIERS` 4096, `MAX_EXPR_NODES` 4096, `MAX_TOTAL_EXPR_NODES`
65536, `MAX_EXPR_DEPTH` 32, `MAX_DERIVED_DEPTH` 128, `MAX_CONDITION_NODES`
256, `MAX_CONDITION_DEPTH` 32. These are bounded-kernel contracts, not
gameplay constants. Every limit needs its largest-valid and first-over-limit
test somewhere in the suites; binary-expression parity puts the node cap
boundaries at 4095/4097.

## Scope and dependencies

One task, this crate only, with T014's explicit lockfile/docs exceptions.
Internal dependency: `crpg-core` only. Approved external dependencies:
workspace `indexmap` and `serde` as normal dependencies; workspace `proptest`
and `serde_json` as dev-dependencies. Implement errors with std; no
`thiserror`, no new feature, no root-manifest change. Keep unsafe forbidden
and missing docs warned; document every public item. If the work needs
another crate's API, a schema change, or an unlisted dependency, stop per
E004 instead of redesigning the contract, weakening a test, or editing a
second crate.

## Definition of done for any change

Focused suites plus doctests and the full gate list from T014, on native
Windows/MSVC and genuine Linux/GNU with the pinned toolchain:

```
cargo test -p crpg-rules --test stats --locked
cargo test -p crpg-rules --test modifier_table --locked
cargo test -p crpg-rules --test modifier_properties --locked
cargo test -p crpg-rules --test derived --locked
cargo test -p crpg-rules --test persistence --locked
cargo test -p crpg-rules --test hooks --locked
cargo test -p crpg-rules --doc --locked
cargo test -p crpg-rules --locked
cargo fmt --all
cargo clippy -p crpg-rules --all-targets -- -D warnings
cargo test -p crpg-rules
python tools/lint/deps.py
python tools/lint/determinism.py
python -m unittest discover -s tools/lint -p "test_*.py"
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo deny check
git diff --check
```

Any `proptest-regressions/` file a failure produces is committed, not
ignored. Do not commit, push, or open a PR from an implementation worktree
without maintainer instruction.

## Agent log

- 2026-09-26 (UTC) · opencode/muse-spark + T014 crate opening · Created this contract before source implementation so the first agent edit lands under the §15.6 rule, pinning the T014 surface, ordering, determinism, and gate obligations.
- 2026-09-26 (UTC) · opencode/muse-spark + T014 implementation alignment · Recorded the tuple-index lint trap found while gating the table suite; no surface or ordering change.
