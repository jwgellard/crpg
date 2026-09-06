# crpg-testkit — agent contract

Scope note: this file describes the T008b harness. Fixture campaigns,
contract conformance suites and any further helpers arrive with the tasks
that need them; they extend this file, following the module docs that
already name their owners.

## Purpose

Design doc: [`docs/architecture/crpg-testkit.md`](../../docs/architecture/crpg-testkit.md)
— what the crate is and how its pieces fit. This file is the working contract:
what you may do and what will break. The dependency rule it follows but does
not ratify is E005; the determinism scope it operates under is ADR-0009; the
mismatch shape is ADR-0010.

Shared test machinery, built once so it cannot fork across crates: the
hash-sequence harness and golden-file convention that T009's replay, T016's
combat and every golden file after them consume. This crate serves tests and
harnesses; it never ships in a binary.

## Public API  (changing this requires an ADR)

`run_hash_sequence`, `ScriptStep`, `write_golden`, `verify_golden`,
`Mismatch`, `HarnessError`.

- `run_hash_sequence(seed, ticks, script) -> Vec<[u8; 32]>`: script step,
  then `tick`, then `state_hash`, every tick, from `World::new(seed)`.
- `ScriptStep = Box<dyn FnMut(&mut World)>`: public sim API only — a script
  needing a private seam proves the sim API is missing something.
- `write_golden(path, hashes)`: lowercase hex, one per line, `#` scope
  headers, trailing newline, parents created.
- `verify_golden(path, hashes) -> Result<(), HarnessError>`: `Ok` on full
  match; `Mismatch` enum on divergence — `Diverged`, `GoldenShort`,
  `RunShort`, `Malformed`, `InvalidUtf8` (ADR-0010, only present sides);
  `Io` on filesystem failure. Headers never compare.

## Invariants

1. **The interleaving is fixed.** Script → tick → hash, in that order, for
   every consumer. It is what makes sequences comparable across tasks.
   Consumers do not reorder it.
2. **No tolerant comparison, ever.** No fuzzy hashes, no per-platform
   goldens chosen at runtime, no header-aware forgiveness beyond skipping
   `#` lines. A cross-platform mismatch is filed against the CI job, never
   "fixed" in the compare function (ADR-0009).
3. **Golden format is append-only in compatibility.** New header lines must
   start with `#`; hash lines stay exactly 64 lowercase hex chars. Old files
   verify under new code or the format change is a migration with its own
   task.
4. **Hex and errors stay hand-rolled.** No `hex` crate, no `thiserror`, no
   `tempfile` — none of them is worth a node in the dependency graph for
   what a dozen lines do. Revisit only with a measured need, not elegance.
5. **This crate never ships.** Nothing here may grow a runtime-only caller;
   anything a binary needs is a different crate's API.
6. `#![forbid(unsafe_code)]`, `#![warn(missing_docs)]`. Every public item
   has a doc comment (spec §15.6).
7. No clock, no threads, no randomness of its own. File I/O is confined to
   the two golden functions; everything else is pure. (This crate is outside
   the determinism lint's crate list, which makes the discipline
   self-imposed — hold it anyway; the day testkit output feeds a golden,
   sloppiness here becomes flakiness everywhere.)

## Allowed dependencies

`crpg-sim` (path, normal), `crpg-core` (path, dev-only for tests).
No workspace crate beyond those two — later helpers (data fixtures, rules
suites) add their edges with their tasks, each justified in the task file.
No external crate beyond what the workspace already holds; anything else
needs approval.

## Definition of done for any change

```
cargo fmt --all
cargo clippy -p crpg-testkit --all-targets -- -D warnings
cargo test -p crpg-testkit
python tools/lint/deps.py
python tools/lint/determinism.py
python -m unittest discover -s tools/lint -p "test_*.py"
```

Any `proptest-regressions/` file a failure produces is **committed**, not
ignored: it is the shrunk counterexample, and losing it loses the regression.

## Known traps

- **The harness does not reach around the sim API.** Missing capability is
  a filed gap against the owning sim task, never a `pub(crate)` favour or a
  second quasi-API. The T008b commit asserts an empty sim diff; keep that
  true.
- **The fixed interleaving is not a suggestion.** A consumer hashing before
  ticking, or ticking twice per script step, produces sequences no other
  task can compare against. The failure mode is silent: everything verifies
  against itself and cross-task comparison quietly means nothing.
- **Missing golden ≠ wrong golden ≠ unreadable text.** `Io(NotFound)` and
  `Mismatch` are different variants on purpose. Do not merge them into "no
  approved baseline" — the first means "write one," the second means
  "behaviour changed," and confusing them wastes exactly the investigation
  the harness exists to shorten. Non-UTF-8 files are `Mismatch::InvalidUtf8`,
  not `Io`.
- **Never fabricate a hash side.** Length and malformed paths carry `None`
  or their own variant (ADR-0010). Zeroed `[0; 32]` sentinels made
  `expected != actual` unreliable and hid the approved hash.
- **Scope discipline belongs to CI.** Filenames carry the full ADR-0009
  scope; comparison runs on canonical Linux per E020's sequencing. The day
  someone proposes choosing goldens by platform at runtime, that proposal
  amends an ADR, not this module.
- **E005 is followed, not ratified.** This crate reaches `sim` and `sim`
  never reaches back. If a future task wants a sim dev-dependency on
  testkit, that task reopens E005 first — the cycle check only covers
  runtime edges, so the lint will not catch it, but cargo will refuse to
  build it.

## Agent log

- 2026-09-06 (UTC) · opencode/muse-spark + T008b · Wrote the crate contract for the harness: API list, seven invariants, and traps for the interleaving, tolerant comparison, missing-vs-wrong goldens, scope discipline and the unratified E005 direction.
- 2026-09-06 (UTC) · opencode/muse-spark + testkit mismatch hardening · Replaced the Mismatch struct with the ADR-0010 enum (truthful sides, UTF-8 as content divergence) and recorded the no-sentinel trap.
