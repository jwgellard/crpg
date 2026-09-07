## Task
Resolve `crpg-testkit` dependency ownership. **Decided 2026-09-06: strict
one-way integration consumer.**

## Original problem

When `crpg-testkit` is implemented (T008–T009 era, "one-way dev-support crate"),
the intended topology created a cycle:

- Every crate dev-depends on `crpg-testkit` (to use its fixtures/matchers).
- `crpg-testkit` depends on `crpg-sim` (to construct Worlds/fixtures).

That is `crpg-sim -> crpg-testkit -> crpg-sim` in the combined graph. Cargo can
compile dev-only cycles of this shape, but permitting the edge would let a
lower layer reach a consumer that knows the completed simulation. The reason
to reject it is architecture and test ownership, not a Cargo limitation.

## Decision
- Testkit may depend down through sim. Core, data, rules, and sim never depend
  on testkit, including as a dev-dependency; their cross-layer integration
  tests live in testkit.
- A higher crate may dev-depend on testkit only when every normal workspace
  dependency testkit brings is already legal for that crate and testkit does
  not depend back on it.
- The explicit `ALLOWED` table enforces actual edges, including dev edges. A
  future task adds a higher-crate edge only with a concrete test consumer; no
  speculative allowlist is opened now.

## Delivered
- Root and testkit `AGENTS.md` record the one-way rule.
- The existing lint scans dev-dependencies through the same explicit `ALLOWED`
  table as normal dependencies. Future higher-crate testkit edges remain
  closed until a concrete task adds one.

## Consequences
- T009a belongs entirely to `crpg-testkit`; no sim-side helper or dev-edge is
  needed.
- Self-contained lower-layer unit/property tests stay close to their code.
- Cargo's runtime-only cycle model is not used as an architecture policy.

## Agent log

- 2026-09-06 (UTC) · opencode/muse-spark + E004 split · The T008 split sidesteps the cycle for now: T008a uses no testkit helper (self-contained `proptest` tests, as T007's were) and T008b is a plain `testkit -> sim` normal dependency, which the ALLOWED table already permits. The general one-way rule still needs ratifying before T009 deepens testkit integration; BACKLOG retargets this task's Blocks column to T009 accordingly.
- 2026-09-06 (UTC) · opencode/gpt-5.6-sol + maintainer decision · Ratified strict one-way ownership, corrected the false Cargo-rejection premise, and assigned lower-layer cross-crate tests to testkit without opening speculative dependency edges.
