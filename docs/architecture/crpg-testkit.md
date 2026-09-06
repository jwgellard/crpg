# crpg-testkit — architecture

Shared test machinery, built once so it cannot fork across crates.

**State:** the hash-sequence harness is complete (T008b): scripted
scenario runs, golden-file write/compare, and the convention later tasks
use. Fixture campaigns, contract conformance suites and further helpers are
planned, each owned by the task that first needs it.

Decisions: [ADR-0009](../adr/0009-determinism-scope.md) for the scope the
harness operates under; E005 (unratified) for the dependency direction it
follows.
Working contract: [`crates/crpg-testkit/AGENTS.md`](../../crates/crpg-testkit/AGENTS.md).

---

## Position

`crpg-testkit` may reach any simulation crate except the Godot bridge — the
one crate in the ALLOWED table with an exclusion instead of a closed set —
because every crate takes testkit as a dev-dependency, and a testkit that
pulled the engine would pull it into all of them. Today it reaches
`crpg-sim` (normal) and `crpg-core` (tests only). Later helpers add edges
with their tasks: data fixtures will want `crpg-data`, rules suites
`crpg-rules`, each justified in its task file.

The direction is one-way by convention pending E005's ratification: testkit
reaches down the stack, and no stack crate reaches back — not even as a
dev-dependency, where the lint's cycle check (runtime edges only) would stay
green while cargo refuses to build. T008b follows the rule without declaring
it; the crate that breaks it will discover the build failure personally.

## Modules

- **`harness` — `run_hash_sequence`, goldens, errors.** The fixed
  script→tick→hash interleaving, the line-hex golden format with `#` scope
  headers, exact-tick mismatch reports. T009's replay and T016's combat are
  named future consumers; the module serves them, it does not anticipate
  them.

## Today versus planned

| Exists (T008b) | Planned (owner) |
|---|---|
| Hash sequences, golden write/compare, error shape | Replay format + divergence diffs (T009) |
| Determinism/tamper/shape tests | Fixture campaigns (first data task needing one) |
| Scope-header convention, CI split documented | Canonical-Linux comparison job (E020) |
| — | Contract conformance suites (spec §15.3, first backend) |

## What consumers inherit

- **Comparable sequences.** Same interleaving everywhere means a hash at
  index *n* means the same thing in every task's test: the world after the
  *n*-th scripted step and tick.
- **Reviewable goldens.** Line-hex under scope headers diffs cleanly; a
  one-tick behaviour change shows as one changed line plus the report naming
  it.
- **No engine in the test graph.** Depending on testkit never drags Godot,
  windowing, or audio into a headless test — the exclusion that keeps CI
  fast is structural, not careful.

## Agent log

- 2026-09-06 (UTC) · opencode/muse-spark + T008b · Wrote the crate doc for the harness: testkit's position and one-way direction, the module map, today-vs-planned with T009/T016/E020 as named consumers, and what consumers inherit.
