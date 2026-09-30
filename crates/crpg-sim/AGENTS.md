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

`World`, `EntityMeta`, `ComponentStore<T>`, `Timeline`, `InitiativeKey`,
`Transform`, `SimEvent`, `tick`, `end_turn`, `state_hash`, plus the T016b
combat surface: `Combatant`, `CombatState`, `CombatDefinition`,
`EncounterSpec`, `PlacementAndArea`, `CombatAction`, `ActionOutcome`,
`CombatError`, `start_encounter`, `perform_action`,
`MAX_COMBATANTS`, `MAX_COMBAT_STATS`, `COMBAT_ROLL_STREAM`,
`COMBAT_ROLL_TAG`, plus the T016f release surface: `ParticipantResult`,
`EncounterSummary`, `end_encounter`, plus the T020 opt-in history surface:
`HistoryWorld`, `HistoryEvent`, `HistoryEnvelope`, `HistoryError`,
`history_hash`, `MAX_HISTORY_EVENTS`, `MAX_HISTORY_BYTES`,
`MAX_HISTORY_PAGE`, `MAX_HISTORY_STRING_BYTES`, `HISTORY_VERSION`.

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
- `start_encounter(world, spec)`: validates an authored encounter
  all-or-nothing (pinned `CombatError` precedence: active-encounter,
  collection bounds, placement/area/creature, per-stat presence/wholeness/
  health-positivity, per-ability dice/table/cost) and only then publishes:
  one entity per participant in authored order, pools at max, timeline at
  authored initiative keys, `round = 0`, `active` = timeline head, the
  `roll` tag interned, the incoming actor refreshed once with `TurnStart`.
- `perform_action(world, action)`: validates in pinned precedence order
  (`NoEncounter` … `InsufficientAction`) before any mutation — including
  before any RNG stream is created — then resolves through the rules
  kernel (`Against::Dc` on the actor's authored attribute, one named
  stream, one interned roll tag, empty pipeline/views), spends the cost,
  applies saturating nonnegative damage flooring at zero, emits `Died`
  exactly once on the positive-to-zero transition, removes dead
  participants from scheduling (entities and zero-health state retained),
  and advances: next head + `TurnStart` refresh, round rollover (round +1,
  `RoundStart` refresh, re-add live participants) when the timeline
  empties with two or more alive, terminal (`active = None`) otherwise. A
  rejected action leaves the whole world — resources, RNG, events, event
  sequence — unchanged. A valid failed attack consumes its action; an
  invalid action consumes nothing. Combat turn logic lives only in these
  two operations, never in `tick` / `run_systems` / `end_turn`. Affordability
  sums same-pool entries as one total in authored template order, primary
  first, with checked `u32` addition (`ValueOverflow`) and `InsufficientAction`
  carrying the total and original balance (T019).
- `end_encounter(world) -> Result<EncounterSummary, CombatError>`: releases
  the active encounter any time it is active (terminal or mid-fight) and
  returns the retained summary (`encounter`/`ruleset` ULIDs, closing
  `round`, per-participant placement/entity/health/dead in authored order).
  `combat = None` fails as `NoEncounter` without mutation; otherwise the
  operation strips combat scheduling and components, keeps every entity
  live, and emits no events. Second encounters reuse content (same spec
  shapes) with fresh runtime ids as a deterministic continuation.
- `HistoryWorld` (T020, ADR-0017): the opt-in wrapper privately owning one
  `World` plus its bounded journal. Immutable queries via `world()` only —
  no `&mut World`, stores, queues, RNG, or free mutating controllers escape
  (pinned by `compile_fail` rustdoc examples) — and typed transactional
  mutations (`spawn`, `despawn`, `tick`, `start_encounter`,
  `perform_action`, `end_encounter`) that stage a full clone, reuse the
  existing controllers, drain staged legacy events exactly once, translate
  each exactly once, collect branch-observed transition facts, validate the
  journal in pinned order (strings, sequence range, event count, canonical
  byte total), and publish only on success. Gameplay errors win before
  history errors; rejection preserves authoritative state, RNG, entity
  allocation, and all counters. Emission order per operation is
  `ActionResolved`, drained `Died`s, then the actual `TurnStarted` or first
  `EncounterEnded`; explicit release journals nothing. `read_after` pages at
  most 256 envelopes with `StaleCursor`/`FutureCursor` errors and inert
  reads; `acknowledge` is monotonic and idempotent with no slow-peer cursor.
  `history_hash` covers the complete canonical wrapper (world, version,
  pending payloads, sequence/ack state) with no exclusions; legacy
  `state_hash`, APIs, emissions, bytes, and hashes are unchanged.

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
9. **`state_hash` checks finiteness first.** `Transform` fields are public
   `f32`, so safe code can build a NaN/infinity. `serde_json` would write
   those as `null` and collide distinct corrupted states, so the hash
   asserts `is_finite` on every position/velocity before serializing.
7. `#![forbid(unsafe_code)]`, `#![warn(missing_docs)]`. Every public item
   has a doc comment (spec §15.6).
8. No `HashMap`/`HashSet` — anywhere, including tests. No clock, no threads,
   no I/O. `DeterministicRng` via `rng_mut` with explicit stream names is
   the only randomness.
10. **Same-pool costs are atomic (T019, ADR-0016 §2).** At the existing
    affordability stage (after actor/target/turn/ability/self-target checks,
    before RNG or mutation) every pool's entries sum as one `u32` total in
    authored template order, primary first, never ULID-sorted. An
    unrepresentable sum returns `ValueOverflow`; a representable total above
    balance returns `InsufficientAction` with the total and original balance;
    the first failing pool in template order wins, so a later overflow never
    replaces an earlier insufficiency. Only fully affordable actions reach
    resolution/spending (exact sums; zero entries harmless); persisted vectors
    keep their order, RNG draw order is unchanged, and the adapter's per-entry
    validation plus load coherence still reject impossible shapes before
    execution.
11. **History is opt-in, transactional, and fully hashed (T020, ADR-0017).**
    The wrapper owns its `World` privately; the only mutations are the typed
    methods above — no store/timeline/RNG manipulation, no `From<World>`,
    no into-inner escape, no low-level pop-only `end_turn`. Turn and terminal
    facts come from the controller branches that fired (same-actor returns
    and saturated-`u32` rollover still start logical turns), never from actor
    comparison or round arithmetic; `CombatState.round` stays a persisted
    saturating `u32`, widened to `u64` only inside history payloads. The
    inner legacy queue is empty at every wrapper boundary with its sequence
    counter retained, never reset. Retention is exactly the contiguous suffix
    `(acknowledged, next_seq)` under 4096 envelopes and one MiB of canonical
    bytes with 256-byte strings and 256-envelope pages; sequences never wrap
    (`u64::MAX` is the exhausted sentinel) and partial appends are forbidden.
    Persistence (`version`, `world`, `acknowledged`, `next_seq`, `pending`;
    adjacent-tag payloads with `type` before `value`) rejects
    duplicate/unknown/missing keys, over-limit shapes, gapped sequences,
    decreasing or future ticks, and nonempty inner queues, decoding payloads
    directly with bounded visitors — retained-history bounds, not a claim
    about parser scratch, whose mandatory pre-parse byte cap belongs to
    T022's host boundary.

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
- **The combat controller never invents actions.** `tick` / `run_systems`
  know nothing of combat; `end_turn` stays pop-only. Refresh happens once
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

## Compatibility with B1 shapes (T017b, specified before source)

The B1 data shapes replace authored `Ruleset.action_pool` with
`Ruleset.pools` and extend `Ability` with `extra_costs`, `ends_turn`,
`effect`, `defense`, and `natural_die`. This task adapts the adapter only:
`start_encounter` reads `ruleset.pools.first()` for the existing
single-pool runtime (`CombatDefinition.pool_*`, `Combatant.action_pool`);
fixtures use legacy-equivalent JSON (`pools: [single]`, `extra_costs: []`,
`ends_turn: true`, no effect, `ActorAttribute`, no selection). Runtime
names, persistence bytes, precedence, and all action/turn/damage/RNG/event
semantics stay unchanged. Multi-pool spending, effects, target defense,
natural selection, and non-ending turns stay in B3. New execution
requirements that cannot be honored must fail before mutation; no existing
error accurately covers them, so that boundary waits on its narrow ADR and
is not implemented here.

As-built (primary part): `combat.rs` reads `pools.first().expect(...)`
matching the existing `abilities[0]` style (data validation guarantees
nonempty; empty-pools clean failure moves to the ADR boundary);
`support/mod.rs` fixtures use the exact current wire; `tests/combat.rs`
gains migrated-v1 and first-listed regressions (38 tests green on
Windows/MSVC). Boundary regressions wait on ADR-0015 approval.

As-built (boundary, ADR-0015 Accepted): `CombatError::UnsupportedAuthored
{ what }` with per-field `spec/<field>: <what>` Display carrying canonical
ULIDs; pinned order encounter→stats→pools→all-abilities→first-support
(`extra_costs, ends_turn, effect, defense, natural_die`); first-only
support with non-first `UnknownAbility` non-executability; six sites plus
empty pools, field-order and multifault regressions (40 combat tests green
on Windows/MSVC). Per-field retirement stays with B3.

## Generalization to multi-ability/pool/effect/turn (T017d, specified before source)

B3 executes the B1 shapes through the production path with no package-name
branches, retiring ADR-0015 field-by-field under accepted ADR-0016:
per-ability definitions addressed by ULID (dice, defense selector, table,
damage, per-pool costs in template order with primary first, `ends_turn`,
optional effect, optional natural selector), plural pool templates carried
in authored order with per-combatant balances (`action_pool` primary
unchanged, `extra_pools` non-primary in template order, skipped when empty)
plus attached `(effect ULID, expires_round)` pairs (ascending effect ULID,
skipped when empty), `EndTurn{actor}` with `perform_action` returning
`Result<Option<ActionOutcome>>` (`Some` attack / `None` `EndTurn`), and
`EncounterSpec.effects` supplying every referenced effect. `InsufficientAction`
gains `pool: Ulid` (`Display` unchanged); six new variants (`MissingEffect`,
`MissingPool`, `InvalidEffect`, `DuplicateModifier`, `InvalidNaturalDie`,
`PolicyConflict`) hide fields in `Display` with pinned `spec/*` locations
and Phase A–E precedence; `UnsupportedAuthored` is deleted when no
restriction remains. Minimal-d6 worlds serialize byte-identically via
absence-skip (legacy fields always persisted from the first-listed entry;
new vectors persisted together only for non-legacy shapes; execution uses
new vectors when present, else the legacy-derived single-entry view).
Neutrality extends to every new/changed file with the second-ruleset
vocabulary added to the banned list.

As-built: `combat.rs` carries the generalized definition/adapter/controller
(`EndTurn`, `Option` return, pool-scoped insufficiency, effect attachment,
natural bound, Phase A–E precedence) with joint absence-skip serialization;
`world.rs` carries the extended coherence checks; `UnsupportedAuthored` is
deleted with six-site execution coverage in the new 14-test `combat_multi`
suite; existing suites pass via mechanical updates only (effects maps,
`Option` unwraps, pool identity, retirement smoke); legacy `combat_basic`
goldens byte-identical per target. The breaking API required a
user-authorized narrow mechanical fix in the downstream adapters
(`crpg-testkit/tests/support/combat.rs`, `crpg-cli/src/combat_apply.rs`:
effects-map wiring plus one `Option` unwrap; no golden/behavior change),
recorded in `tasks/T017d.md`; B4 still owns replay/goldens and B5 the CLI
proof.

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
