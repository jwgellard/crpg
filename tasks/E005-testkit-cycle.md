## Task
Resolve the `crpg-testkit` dependency-cycle risk. Human-decision task.

## Why this is deferred

When `crpg-testkit` is implemented (T008–T009 era, "one-way dev-support crate"),
the intended topology created a cycle:

- Every crate dev-depends on `crpg-testkit` (to use its fixtures/matchers).
- `crpg-testkit` depends on `crpg-sim` (to construct Worlds/fixtures).

That is `crpg-sim -> crpg-testkit -> crpg-sim` in the dev graph. Cargo rejects
dev-dependency cycles, so this must be resolved by a one-way ownership rule.

## Decision to make
- Confirm "testkit depends on sim; sim (and every crate below sim) does NOT
  dev-depends on testkit" — i.e. the dependency direction mirrors the runtime
  graph, and testkit may depend only on crates at its own level or below.
- Specify exactly which crates may dev-depend on testkit (rules, sim, script,
  net, persist) and which may not (core, contracts, edit, cli/server).
- Encode this in AGENTS.md and, if the lint grows a dev-graph mode, in
  `tools/lint/deps.py`.

## Deliverable
- A note in AGENTS.md or an ADR recording the one-way rule.
- Later enforced in the lint when the dev graph is modeled.

## Constraints
- Human decision; do not guess. This blocks T009-style integration testing.

## Agent log

- 2026-09-06 (UTC) · opencode/muse-spark + E004 split · The T008 split sidesteps the cycle for now: T008a uses no testkit helper (self-contained `proptest` tests, as T007's were) and T008b is a plain `testkit -> sim` normal dependency, which the ALLOWED table already permits. The general one-way rule still needs ratifying before T009 deepens testkit integration; BACKLOG retargets this task's Blocks column to T009 accordingly.