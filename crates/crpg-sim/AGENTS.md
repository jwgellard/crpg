# crpg-sim — agent contract

Scope note: this file describes the skeleton (T007) plus the loop and the
instrument (T008a). Movement and every further component arrive in later
tasks; they extend this file, following the module docs that already name
their owner.

## Purpose

Design doc: [`docs/architecture/crpg-sim.md`](../../docs/architecture/crpg-sim.md)
— what the crate is and how its pieces fit. This file is the working contract:
what you may do and what will break. Decisions live in ADR-0006, ADR-0007,
ADR-0008 and ADR-0009. Keep them linked rather than copied.

One loaded area's simulation state and the operations that keep it coherent:
`World` (entity arena, component stores, timeline container, live event
queue, deterministic RNG, tick counter), `ComponentStore<T>`, `Timeline`,
`Transform`, `SimEvent`, the spawn/despawn/query surface, and the fixed-step
`tick` / `end_turn` advance plus the `state_hash` instrument.

## Public API  (changing this requires an ADR)

`World`, `EntityMeta`, `ComponentStore<T>`, `Timeline`, `InitiativeKey`,
`Transform`, `SimEvent`, `tick`, `end_turn`, `state_hash`.

- `World::new(seed)`, `spawn(meta) -> EntityId`, `despawn(id) -> bool`,
  `contains`, `len`, `is_empty`, `ids`, `tick` (getter only),
  `transforms` / `transforms_mut`, `timeline` / `timeline_mut`,
  `events` / `events_mut`, `rng_mut`, `Default` (= `new(0)`),
  `Serialize` / validated `Deserialize` (skeleton only, see E014 trap below;
  loading rejects dangling transforms/timelines and duplicate timeline
  entries).
- `ComponentStore<T>`: `new`, `insert` (returns replaced), `remove`, `get`,
  `get_mut`, `contains`, `len`, `is_empty`, `clear`, `iter`, `iter_mut`.
- `Timeline`: `new`, `insert` (replaces the entity's entry), `remove`
  (O(n)), `contains`, `len`, `is_empty`, `iter` ascending.
  `From<Vec<_>>` applies the same replace rule (last wins);
  `Deserialize` rejects duplicate entities instead of normalizing.
- `InitiativeKey(pub i32)`, `Transform { position, velocity }`,
  `SimEvent::{Spawned, Despawned}`, `EntityMeta {}` (reserved placeholder).
- `tick(world)`: counter saturating +1, then the ordered system list (today
  exactly `[timeline_system]`). `end_turn(world)`: pops the timeline head,
  never re-queues, never despawns. `state_hash(world) -> [u8; 32]`: BLAKE3
  over canonical JSON, queue bytes included, exclusions none.

## Invariants

1. **Despawn is total, and loading is validated.** A successful `despawn`
   removes the entity, strips every store and the timeline entry, and
   enqueues exactly one `Despawned`. A dead id returns `false` and enqueues
   nothing. After any op sequence, no store key and no timeline entry
   addresses a non-live entity —
   `tests/world.rs::assert_no_dangling` checks this on every generated world.
   Deserialization enforces the same shape: dangling transforms/timelines
   and duplicate timeline entities fail to load.
2. **Events are automatic and tick-stamped.** `spawn`/`despawn` enqueue at the
   world's current tick. Draining happens in the tick loop; time advances
   only through `tick` / `end_turn`.
3. **One timeline entry per entity.** `insert` and `From<Vec<_>>` replace
   (last wins). `remove` is total because live timelines hold no duplicates;
   malformed serialized timelines with duplicates are rejected on load.
4. **Iteration contracts are load-bearing.** Arena iteration is ascending
   index (inherited); store iteration is insertion order (documented, not
   sorted); timeline iteration is ascending `(key, id)`. Determinism tests
   pin all three across identical sequences.
5. **`Transform` is `f32`-only (E006-A).** Positions and velocities, never
   rules input. No `f64` anywhere in the crate.
6. **Serde is pair-lists where JSON needs strings, with load-time
   guards.** `ComponentStore` and `Timeline` serialize as `(id, value)` /
   `(key, id)` lists because struct keys are not JSON keys. `EntityMeta` is
   braced (`{}`), not a unit, because a unit serializes as `null` —
   indistinguishable from a vacant arena slot. `World` deserialization
   rejects non-live component/timeline ids; `Timeline` deserialization
   rejects duplicate entities.
9. **`state_hash` checks finiteness first.** `Transform` fields are public
   `f32`, so safe code can build a NaN/infinity. `serde_json` would write
   those as `null` and collide distinct corrupted states, so the hash
   asserts `is_finite` on every position/velocity before serializing.
7. `#![forbid(unsafe_code)]`, `#![warn(missing_docs)]`. Every public item
   has a doc comment (spec §15.6).
8. No `HashMap`/`HashSet` — anywhere, including tests. No clock, no threads,
   no I/O. `DeterministicRng` via `rng_mut` with explicit stream names is
   the only randomness.

## Allowed dependencies

`crpg-core` (path), `indexmap`, `serde`, `serde_json`, `blake3` (T008a:
hash serializes through `serde_json`, digests with `blake3`). Dev-only:
`proptest`. No workspace crate beyond `crpg-core` — the `crpg-data` and
`crpg-rules` edges in the ALLOWED table are for later tasks, not
speculative imports. Anything else needs approval.

## Definition of done for any change

```
cargo fmt --all
cargo clippy -p crpg-sim --all-targets -- -D warnings
cargo test -p crpg-sim
python tools/lint/deps.py
python tools/lint/determinism.py
python -m unittest discover -s tools/lint -p "test_*.py"
```

Any `proptest-regressions/` file a failure produces is **committed**, not
ignored: it is the shrunk counterexample, and losing it loses the regression.

## Known traps

- **The stores do no liveness checking.** `transforms_mut().insert(dead_id,
  ...)` succeeds, and `timeline_mut().insert(key, dead_id)` succeeds. The
  no-dangling invariant is `World::despawn`'s job for live ops and `World`
  deserialization's job for loaded state — the store has no arena to check
  against. Do not add store-level checks; do not bypass `despawn` when
  removing entities.
- **`iter` is insertion-ordered, not sorted.** After removals, order is still
  deterministic for identical sequences but is not ascending. If sorted
  iteration is ever needed, add a method with its own test.
- **`Timeline::remove` is O(n) on purpose.** An index is a second structure
  that can disagree with the first. A perf task may add one with a
  consistency test; do not "optimise" it with an untested side table.
- **No `StatId`-keyed map may gain a derived serde (E014).** When `StatBlock`
  arrives (T014), its persisted form is the string conversion pair. Deriving
  `Serialize` on a struct holding interned handles reintroduces the
  renumbering bug ADR-0006 exists to prevent.
- **No policy in the timeline module.** No `advance`, no `next_turn`. The
  advance rules are T008's tick loop (E015). The container/advance split is
  deliberate and reviewed.
- **No new `SimEvent` variants without consumers.** Each variant is a
  vocabulary decision with a producer and a reader. Speculative variants are
  how game churn reaches the skeleton.
- **`World::tick` is a getter.** Time advances only through `tick` /
  `end_turn` (T008a). Do not add a setter to "help" a test; construct the
  world you need.
- **Tick order is serial territory (spec §15.2).** `run_systems` is a
  hand-written list in spec §10 stage order; appending is routine,
  reordering is a behaviour change reviewed as one. No scheduler, no
  parallelism, no per-entity effect smuggled into `timeline_system` without
  its task.
- **Hash exclusions need behaviour-proof tests.** The list is empty and
  stays empty until a test proves the excluded field cannot affect
  behaviour. Category (b) admissions and rule changes are ADR-level
  (ADR-0009).
- **Determinism scope is ADR-0009.** Same binary + same inputs ⇒ same
  hashes; cross-platform and cross-build sameness are explicitly not owed.
  A hash divergence outside the exact-build scope is out-of-scope by
  citation, not a bug.
- **`state_hash` panics on non-finite floats by design.** Public `f32`
  fields can hold NaN/infinity, so the hash asserts `is_finite` up front:
  `serde_json` would emit `null` for those and collide distinct corrupted
  states. A non-finite in state means corruption or bridge abuse. Do not
  add a scrubbing pass; fix the producer.
- **`From<Vec<_>>` normalizes, `Deserialize` rejects.** `Timeline::from`
  keeps the last entry per entity so the infallible conversion preserves
  the replace rule; serialized input with duplicates is corrupt and fails
  to load. Do not route deserialization through `From`.

## Agent log

- 2026-09-06 (UTC) · opencode/muse-spark + T007 · Wrote the crate contract for the skeleton: API list, eight invariants, and traps for liveness, iteration order, E006-A/E014/E015 boundaries and the tick getter.
- 2026-09-06 (UTC) · opencode/muse-spark + T008a · Extended the API with `tick`/`end_turn`/`state_hash` and added traps for tick order, hash exclusions, ADR-0009 scope and the designed float panic.
- 2026-09-06 (UTC) · opencode/muse-spark + sim invariant hardening · Corrected the scope note, runtime deps and float-panic premise; recorded validated World/Timeline deserialization and the From-normalizes-vs-Deserialize-rejects split.
