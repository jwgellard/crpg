# crpg-rules — agent contract

Read the root rules and [T014](../../tasks/T014.md) plus [T015](../../tasks/T015.md),
the binding kernel API and acceptance contracts. Architecture:
[crpg-rules](../../docs/architecture/crpg-rules.md). This document describes
the T014 stat/modifier kernel and the T015 dice/outcome/resolution surface;
results live in T014 and T015 respectively.

## Public surface

Public modules `stats`, `modifier`, `derived`, `hooks`, `error`, plus the
T015 modules `dice`, `resolution`, and `resource`, re-export their public
items at the root. T014 lists the stat/modifier kernel shapes; T015 lists
every additional required type, field, variant, function signature, and
limit constant. Operations added by T015 are `DiceExpr::{new, evaluate}`,
`ModifierPipeline::query_numeric`, `OutcomeTable::{new, evaluate}`,
`resolve`, and `ResourcePool::{new, spend, refresh, can_afford}`, plus the
`StatValue::Dice`/`StatKind::Dice` typing and the four resolution hook
variants. No new trait except implementations of standard/existing traits;
no handler registry or dispatcher, no production JSON writer, no second
public abstraction.

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
  Dice values persist as canonical notation strings and are parsed during
  preflight, before any interning. Never serialize numeric handles; never
  add serde to core handle types.
- Querying a dice-valued stat returns the expression, never a random result.
  Randomness is consumed only by `DiceExpr::evaluate` and `resolve`, which
  draw from caller-selected named streams on the caller's `DeterministicRng`.
  Failed calls leave the complete RNG unchanged, including created streams:
  `resolve` draws on a staged clone and commits only after complete success.
  Successful no-roll resolution mutates no RNG state. Resource pools own no
  timeline and subscribe to no hooks; the host delivers each refresh event.

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
  level, or die assumptions in production or tests. Configure face values
  and outcome labels in data; production holds no game-system branches.
- Dice selection ranks equal values by earlier original index first, for
  both keep and drop modes; trace order always stays draw order.
- The literal-exclusion gate forbids one specific die-size notation string
  anywhere under `crates/crpg-rules`, including comments and fixtures: never
  write the banned two-letter die prefix directly followed by that size, and
  never build it by adjacency (a size with that prefix, a prefixed wildcard,
  or a split literal that reassembles it). Construct that size numerically
  (`DiceExpr::new` with an integer) and build its notation at runtime with
  `format!` when a string is needed. Size coverage must still include it.
- Distribution tests stay integer-only: fixed seeds, fixed streams, exact
  face-count bounds. No floating statistics and no runtime entropy.

## Limits

`MAX_STATS` 1024, `MAX_TAGS` 1024, `MAX_ENUM_VARIANTS` 1024, `MAX_POLICIES`
1024, `MAX_MODIFIERS` 4096, `MAX_EXPR_NODES` 4096, `MAX_TOTAL_EXPR_NODES`
65536, `MAX_EXPR_DEPTH` 32, `MAX_DERIVED_DEPTH` 128, `MAX_CONDITION_NODES`
256, `MAX_CONDITION_DEPTH` 32, plus the T015 bounds `MAX_DICE_INPUT_BYTES`
128, `MAX_DICE_COUNT` 1024, `MAX_DIE_SIDES` 1_000_000 (`u32`), `MAX_OUTCOME_BANDS`
256, `MAX_NATURAL_RULES` 256, `MAX_RNG_STREAM_BYTES` 256. These are
bounded-kernel contracts, not gameplay constants. Every limit needs its
largest-valid and first-over-limit test somewhere in the suites; binary-expression parity puts the node cap
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
Windows/MSVC and genuine Linux/GNU with the pinned toolchain. T015 adds the
`dice`, `dice_properties`, `resolution`, and `resources` suites and the
task's literal-exclusion ripgrep gate over `crates/crpg-rules` (exact
invocation in T015; exit 1 means the banned notation string is absent and
exit 2 is an error). Do not quote that notation string in any file under
`crates/crpg-rules`, including this one; run the invocation from the task
file verbatim in the shell instead:

```
cargo test -p crpg-rules --test dice --locked
cargo test -p crpg-rules --test dice_properties --locked
cargo test -p crpg-rules --test resolution --locked
cargo test -p crpg-rules --test resources --locked
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
(run the literal-exclusion invocation from T015 verbatim in the shell)
```

Any `proptest-regressions/` file a failure produces is committed, not
ignored. Do not commit, push, or open a PR from an implementation worktree
without maintainer instruction.

## Agent log

- 2026-09-26 (UTC) · opencode/muse-spark + T014 crate opening · Created this contract before source implementation so the first agent edit lands under the §15.6 rule, pinning the T014 surface, ordering, determinism, and gate obligations.
- 2026-09-26 (UTC) · opencode/muse-spark + T014 implementation alignment · Recorded the tuple-index lint trap found while gating the table suite; no surface or ordering change.
- 2026-09-26 (UTC) · opencode/muse-spark + T015 crate opening · Extended the surface, randomness/pool rules, determinism traps (selection tie order, literal exclusion, integer-only distribution coverage), new limits, and new gates before source implementation; T014 ordering and ownership semantics preserved.
