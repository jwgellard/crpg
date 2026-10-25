# crpg-sim — agent contract

Scope note: this file describes the skeleton (T007), the loop and the
instrument (T008a), and authoritative headless combat (T016b, T016e–f).
Movement and every further component arrive in later tasks; they extend
this file, following the module docs that already name their owner.

## Purpose

Design doc: [`docs/architecture/crpg-sim.md`](../../docs/architecture/crpg-sim.md)
— what the crate is and how its pieces fit. This file is the working contract:
what you may do and what will break. Decisions live in ADR-0006, ADR-0007,
ADR-0008, ADR-0009, ADR-0013, ADR-0014, ADR-0016, and ADR-0017. Keep them linked rather than copied.

One loaded area's simulation state and the operations that keep it coherent:
`World` (entity arena, component stores, timeline container, live event
queue, deterministic RNG, tick counter), `ComponentStore<T>`, `Timeline`,
`Transform`, `SimEvent`, the spawn/despawn/query surface, and the fixed-step
`tick` / `end_turn` advance plus the `state_hash` instrument — plus the
T016b combat layer: `Combatant` / `CombatState` / `CombatDefinition`
authoritative encounter state, the `start_encounter` / `perform_action`
controller over the ascending `(InitiativeKey, EntityId)` ordering, and the
`Died` vocabulary.

## Public API  (changing this requires an ADR)

Full list: `docs/architecture/crpg-sim.md` §Public API. Treat any change as ADR-level.

## Invariants

1. **Despawn is total, and loading is validated.** A successful `despawn`
   removes the entity, strips every store and the timeline entry, and
   enqueues exactly one `Despawned`. A dead id returns `false` and enqueues
   nothing. Despawning the last combatant releases the encounter (`combat`
   back to `None`; interners, events, RNG, and tick retained) so the world
   still reloads. After any op sequence, no store key and no timeline entry
   addresses a non-live entity —
   `tests/world.rs::assert_no_dangling` checks this on every generated world.
   Deserialization enforces the same shape: dangling transforms/timelines
   and duplicate timeline entities fail to load, and combat saves carry the
   full coherence check — interned roll tag present, declared checked
   attribute and health-stat discipline, fitting costs, per-combatant
   health/dead/maximum agreement with pool/attribute/placement identity, and
   active/timeline scheduling against live non-dead combatants with the
   active turn holding the timeline head.

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

7. `#![forbid(unsafe_code)]`, `#![warn(missing_docs)]`. Every public item
   has a doc comment (spec §15.6).

8. No `HashMap`/`HashSet` — anywhere, including tests. No clock, no threads,
   no I/O. `DeterministicRng` via `rng_mut` with explicit stream names is
   the only randomness. One recorded exception (T097): the `#[cfg(test)]`
   `thread_local!` `ENCODINGS` counter in `src/history.rs` is a
   per-test-thread count of envelope encodings — a counter, not a thread the
   simulation owns, and test-only. It must stay spelled `thread_local!`;
   `std::thread_local!` would read as a thread to `determinism.py`.

9. **`state_hash` checks finiteness first.** `Transform` fields are public
   `f32`, so safe code can build a NaN/infinity. `serde_json` would write
   those as `null` and collide distinct corrupted states, so the hash
   asserts `is_finite` on every position/velocity before serializing.

10. **Same-pool costs are atomic (T019, ADR-0016 §2).** Full invariant: `docs/architecture/crpg-sim.md` §Atomic same-pool affordability (T019).
11. **History is opt-in, transactional, and fully hashed (T020, ADR-0017).** Full invariant: `docs/architecture/crpg-sim.md` §Opt-in authoritative history (T020).
12. **One legality source (T028, ADR-0018).** Full invariant: `docs/architecture/crpg-sim.md` §Shared read-only combat legality (T028).
13. **Area identity is world-owned and immutable (T027a, ADR-0020).** Full invariant: `docs/architecture/crpg-sim.md` §Area identity and explicit transfer (T027a).
14. **The journal's byte total is cached, and `commit` is its only writer (T097, J1).** Full invariant: `docs/architecture/crpg-sim.md` §Journal byte accounting and world-only staging (T097).

## Allowed dependencies

`crpg-core`, `crpg-data`, `crpg-rules` (all path), `indexmap`, `serde`,
`serde_json`, `blake3` (T008a: hash serializes through `serde_json`,
digests with `blake3`). Dev-only: `proptest`. The `crpg-data` and
`crpg-rules` runtime edges are T016b's (dependency approval recorded in
[tasks/T016b.md](../../tasks/T016b.md); ADR-0013 governs the combat API
and state). Anything else needs approval.

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
- **Combat state is world-owned and symbolically persisted.** `combatants`,
  `combat`, and `interners` live in `World` and hash in full; no
  authoritative mutable state sits in caller closures or un-hashed side
  structures. Persistence stores symbolic strings (stat names, dice
  notation, the ordered interner list) — never interned handles — so a
  differently ordered interner reproduces identical behavior (E014).
  Absence-skip (`is_empty` / `is_none`) keeps non-combat worlds
  byte-identical to the T007/T008a skeleton; it is not a hash exclusion.
- **The combat controller never invents actions (turn-based).** In TB,
  `tick` / `run_systems` know nothing of combat; `end_turn` stays pop-only.
  Since T138a the RT stages are the one exception: stage 2 (clock) and stage 3
  (`timeline_system`) execute only orders that commands stored earlier, in an
  RT encounter, and never invent an action. Combat turn logic for TB still
  lives only in `perform_action` and `despawn`; RT logic lives only in
  `combat.rs` (`clock_stage`, `execution_stage`) and `tick.rs` merely calls it.
  Refresh happens once
  per logical turn start via the incoming actor's `TurnStart` (plus
  `RoundStart` on rollover); a rejected action advances nothing.
- **Death is not despawn; release is neither.** A dead combatant keeps its entity, its
  `Combatant` (`health == 0`, `dead == true`), and terminal state; only
  its scheduling entry is removed. `despawn` strips the combatant
  component with the rest: removing the active combatant advances (new
  head + `TurnStart`, rollover, or terminal) through the shared helper,
  removing any other leaves the turn, and removing the last releases the
  encounter. `end_encounter` releases without despawning anyone: entities
  stay live, the summary retains the outcome, and no event is emitted.
  `Died` is the only combat event: core-closed (`entity` only),
  produced exactly once on the positive-to-zero transition, consumed by
  terminal assertions and future AI/effect systems. No hook dispatcher,
  no effect interpreter, no ruleset identifiers in production code.
- **Combat saves fail fast at load, never at action time.** The controller's
  `expect` sites (roll tag, checked attribute, scheduled combatants) are
  unreachable for any loaded or adapter-built world because deserialization
  rejects the incoherent save first. Legal combatant state is exact:
  `max_health >= 1`, `health <= max_health`, `dead == (health == 0)`.
- **The journal's byte total is a cached side value, never a second source of
  truth (T097, J1).** The envelopes are the journal; the total is derived from
  them and `commit` is the only writer of the journal, the owned world and
  `next_seq`. A mutation must encode each new envelope exactly once and never
  re-encode a retained one: a new encode site is a regression, and
  `history::tests::every_envelope_encoding_site_is_accounted_for` fails on it
  until both the test and the contract change. Do not "repair" a stale cache
  by rescanning — fix the writer (the one exception is T6's deliberate
  off-invariant cache, which exists to prove the rejection).

## Compatibility with B1 shapes (T017b, specified before source)

See `docs/architecture/crpg-sim.md` §Compatibility with B1 shapes (T017b).

## Generalization to multi-ability/pool/effect/turn (T017d, specified before source)

See `docs/architecture/crpg-sim.md` §Generalization to multi-ability/pool/effect/turn (T017d, specified before source).

## Kernel range (T137)

See `docs/architecture/crpg-sim.md` §Kernel range (T137, ADR-0029 Follow-on 4).

## Agent log

- 2026-09-06 (UTC) · opencode/muse-spark + T007 · Wrote the crate contract for the skeleton: API list, eight invariants, and traps for liveness, iteration order, E006-A/E014/E015 boundaries and the tick getter.
- 2026-09-06 (UTC) · opencode/muse-spark + T008a · Extended the API with `tick`/`end_turn`/`state_hash` and added traps for tick order, hash exclusions, ADR-0009 scope and the designed float panic.
- 2026-09-06 (UTC) · opencode/muse-spark + sim invariant hardening · Corrected the scope note, runtime deps and float-panic premise; recorded validated World/Timeline deserialization and the From-normalizes-vs-Deserialize-rejects split.
- 2026-09-26 (UTC) · opencode/muse-spark + T016b crate opening · Extended the API with the combat surface (adapter, controller, Died vocabulary), the world-owned symbolic-persistence and absence-skip invariants, the no-invented-action turn rule, and the death-vs-despawn split, with the data/rules runtime edges and ADR-0013 recorded before source implementation.
- 2026-09-27 (UTC) · opencode/muse-spark + T016e invariant repair · Extended loading to the full combat coherence check (roll tag, definition, health/dead/maximum, pool/attribute/placement identity, active-head scheduling) and despawn of the last combatant to encounter release, with the fail-fast-at-load trap recorded above.
- 2026-09-27 (UTC) · opencode/muse-spark + T016f crate opening · Extended the API with the release surface (`end_encounter`, `EncounterSummary`, `ParticipantResult`), the despawn-active advance-or-terminal rule through the shared helper, and ADR-0014 recorded before source implementation.
- 2026-09-27 (UTC) · opencode/muse-spark + T017b crate opening · Extended the contract with the primary-pool adapter rule (pools.first, legacy-equivalent fixtures, unchanged runtime) and held the unsupported-content boundary behind its narrow ADR before source implementation.
- 2026-09-27 (UTC) · opencode/muse-spark + T017b primary implementation · Aligned the adapter with pools.first and legacy fixtures plus migrated-v1/first-listed regressions; boundary work waits on ADR-0015 approval with no new API implemented.
- 2026-09-27 (UTC) · opencode/muse-spark + T017b boundary implementation · Implemented the accepted UnsupportedAuthored gate with pinned order/Display/field-order/multifault coverage; per-field retirement stays with B3, no generalization added.
- 2026-09-27 (UTC) · opencode/muse-spark + T017d crate opening · Extended the contract with the accepted multi-ability/pool/effect/turn generalization (per-ability ULID definitions, plural pools, EndTurn with Option return, six new variants, Phase A–E precedence, absence-skip hash preservation, per-field retirement) under ADR-0016 before source implementation.
- 2026-09-27 (UTC) · opencode/muse-spark + T017d implementation · Aligned the contract with the as-built generalization, joint absence-skip persistence, per-field retirement with combat_multi coverage, and the user-authorized narrow downstream mechanical fix; no commit/push/PR.
- 2026-09-28 (UTC) · opencode/muse-spark + T019 implementation · Recorded the atomic same-pool affordability invariant (template-order sums, checked overflow, first-failing wins, exact-sum spending) with the perform_action clarification above; no API, dependency, fixture, or golden change.
- 2026-09-29 (UTC) · opencode/muse-spark + T020 implementation · Extended the API with the opt-in history surface (privately owned world, transactional typed mutations, bounded read/ack journal, full-wrapper hash) and the branch-observed transition-fact and retained-vs-parser-memory invariants above; legacy APIs, emissions, bytes, and hashes unchanged, no dependency added.
- 2026-09-30 (UTC) · claude-code + T028 implementation · Added the validate_action/legal_actions surface and the one-legality-source invariant so later rules land in the shared validator instead of a divergent copy; no dependency, persisted field, or golden change.
- 2026-09-30 (UTC) · claude-code + T027a implementation · Added the persisted area identity and dual-world transfer surface with its world-owned, immutable-identity invariant, so later components know they must join the move contract rather than vanish on transfer.
- 2026-10-17 (UTC) · opencode-go/deepseek-v4.1-flash + T097 implementation · Recorded the cached journal byte total and world-only staging as invariant 14, named `commit` the only journal/world/sequence writer, added the derived-side-value trap and the source guard that enforces it, and amended rule 8 for the test-only `ENCODINGS` counter; no bytes, hashes, goldens, or public API changed.
- 2026-10-21 (UTC) · opencode-go/deepseek-v4.1-flash + T133 · Restored T097 D2's qualifier that `commit` is the journal's only writer before validation ends, and added the 4095/4096-envelope `ENCODINGS` and journal-in-place regressions next to the existing small-journal tests; no behaviour, public API, or byte changed.
- 2026-10-23 (UTC) · opencode-go/deepseek-v4.1-flash + T136c · Recorded the temporary T136b bridge (`UnsupportedAuthored` before placement, unreachable definition arms, one test per trigger) and that T137 deletes it; no existing error precedence, golden, or hash changed.
- 2026-10-23 (UTC) · claude-code (Sonnet 5.5) + T138a · Added the encounter-mode surface and RT tick stages to the API list, and amended the "tick knows nothing of combat" trap, which now holds for turn-based play only because RT stages execute stored orders.
- 2026-10-24 (UTC) · claude-code (Sonnet 5.5) + T138a resume · Re-wrapped the amended trap and recorded that `start_encounter_tracked` keeps its two-argument form, with the mode-taking `start_encounter_in_mode_tracked` beside it, because `src/history/tests.rs` is a second caller outside this package.
- 2026-10-24 (UTC) · claude-code (Sonnet 5.5) + T138b · Added the history v2 surface and emission order to the API list, and recorded the lazy sticky wrapper version and the v1 loader rejections, so TB-only histories are known to keep version 1 and their goldens.
- 2026-10-24 (UTC) · claude-code (Sonnet 5.5) + T137a · Replaced the bridge section with the kernel-range section (public additions, the `Int`-only rule, the scratch-interner trap, the B21 start order) and added the stat evaluation stage to invariant 12, so later tasks extend the one legality source instead of a copy.
- 2026-10-24 (UTC) · claude-code (Sonnet 5.5) + T137b · Removed the bridge text from the kernel-range section and added the `Opposed`/damage-dice/initiative draw order, the tag carriage rule and the new start-order steps, so the section describes the whole executed surface and warns against a new refusal list.
- 2026-10-24 (UTC) · opencode-go/beam-free + T137c · Added the surface golden paragraph with the pinned seed-42 schedule, example, test and Windows baseline, so the generalized surface carries a per-target golden with a hand-written value oracle.
- 2026-10-25 (UTC) · opencode-go/beam-free + A0-8 · Trimmed AGENTS.md to how-to-work; moved design/behaviour/history content to docs/architecture/crpg-sim.md.
