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
`MAX_HISTORY_PAGE`, `MAX_HISTORY_STRING_BYTES`, `HISTORY_VERSION`, plus the
T028 legality surface (ADR-0018): `validate_action`, `legal_actions`,
`LegalActionsError`, `MAX_LEGAL_ACTIONS`, plus the T027a area surface
(ADR-0020): `World::new_in_area` / `area` / `area_of`,
`HistoryWorld::new_in_area`, `AreaError`, `transfer_entity`,
`transfer_history_entity`, and `CombatError::AreaMismatch`, plus the T138a
encounter-mode surface (ADR-0029): `EncounterMode`, `EncounterClock`,
`StandingOrder`, `RT_CLOCK_MAX`, `start_encounter_in_mode`, `request_mode`,
`CombatAction::Stop`, and `CombatError::{ModeUnsupported, WrongMode}`, plus
the T138b history v2 surface: `HistoryWorld::{version, start_encounter_in_mode,
request_mode}`, the `HistoryEvent` variants `RoundStarted`, `ModeChanged` and
`OrderEnded`, and `HISTORY_VERSION` now 2 (1 is still read). T139c adds the variants `Joined` and `Left` (the last two of five
version 2 variants) and `HistoryWorld::{start_encounter_with_members,
join_encounter, leave_encounter}`.

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
  invalid action consumes nothing. Turn-based combat turn logic lives
  only in these two operations, never in `tick` / `run_systems` / `end_turn`
  (RT stages 2 and 3 execute stored orders only, T138a). Affordability
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
  `perform_action`, `end_encounter`) that stage a clone of the world, reuse
  the existing controllers, drain staged legacy events exactly once,
  translate each exactly once, collect branch-observed transition facts,
  validate the journal additions against the live wrapper in pinned order
  (strings, sequence range, event count, cached canonical byte total), and
  publish only on success. Gameplay errors win before
  history errors; rejection preserves authoritative state, RNG, entity
  allocation, and all counters. Emission order per operation is
  `ActionResolved`, drained `Died`s, then the actual `ModeChanged`,
  `RoundStarted`, and `TurnStarted` or first `EncounterEnded` (T138b); an RT
  `tick` journals `ModeChanged`, `RoundStarted`, `TurnStarted`, then each
  `ActionResolved` followed at once by its `Died` and each `OrderEnded`, then
  `EncounterEnded`; explicit release, RT order setting, `Stop` and
  `request_mode` journal nothing. The wrapper `version` is 1 until the first
  `commit` whose staged world holds a clock or whose events include a v2
  variant, then 2 for good, and 3 from the first committed `Moved` (T036c;
  `commit` is its only writer); the loader rejects a version-1 wrapper
  holding a v2 event or a clock, a version below 3 holding a `Moved`, and any
  version other than 1, 2 or 3. A history change that moves a `history_v2` or `history_v1`
  golden is a stop, not a re-bless. `read_after` pages at
  most 256 envelopes with `StaleCursor`/`FutureCursor` errors and inert
  reads; `acknowledge` is monotonic and idempotent with no slow-peer cursor.
  `history_hash` covers the complete canonical wrapper (world, version,
  pending payloads, sequence/ack state) with no exclusions; legacy
  `state_hash`, APIs, emissions, bytes, and hashes are unchanged.

- `validate_action(world, action)` / `legal_actions(world, actor)` (T028,
  ADR-0018): `validate_action` is the only pre-mutation admission path —
  `perform_action` calls it first — and changes nothing (no interning, no
  RNG stream, no event, no spend, no advance). `legal_actions` returns
  `Actor(error)` when `EndTurn { actor }` fails, otherwise every accepted
  `UseAbility` over abilities by ascending ULID × current combatants by
  ascending full `EntityId`, then `EndTurn`; each candidate's own failure
  excludes only that candidate. Over `MAX_LEGAL_ACTIONS` (4096, `EndTurn`
  counted) the whole query fails with `TooManyOptions`, never a partial list.
- Area identity (T027a, ADR-0020): `World::new_in_area(seed, area)` binds an
  immutable authored area ULID (no setter, no late binding); `new`/`Default`
  stay unbound and byte-identical (`area` omitted when `None`; missing or
  `null` loads as `None`). `area_of` is the world's area iff the full id is
  live — death and release keep presence, despawn and slot reuse do not
  restore it. A bound `start_encounter` returns `AreaMismatch` after the
  `MissingPlacement`/`MixedArea` checks and before creature/stat checks.
  `transfer_entity` / `transfer_history_entity` move one noncombat,
  unscheduled entity (`EntityMeta` plus optional finite `Transform`) between
  two bound worlds with a fresh destination id, checking `UnboundSource`,
  `UnboundDestination`, `SameArea`, `AbsentEntity`, `CombatParticipant`,
  `ScheduledEntity`, `InvalidTransform` in order, then (history pair only)
  `SourceHistory` before `DestinationHistory`; any failure publishes neither
  side.

- Encounter mode (T138a, ADR-0029): `CombatState.mode` (TB or RT; hashed;
  omitted when TB), `clock` (`None` = TB-only), `pending_mode`, and
  `Combatant.order`. `start_encounter` is TB (and `start_encounter_tracked` keeps its two-argument
  form, delegating to the mode-taking `start_encounter_in_mode_tracked`); `start_encounter_in_mode`
  publishes RT (every key `0`, `active = None`). In RT, `validate_action`
  swaps only its turn stage (terminal gate for `UseAbility`/`Stop`,
  `WrongMode` for `EndTurn`; `Stop` in TB is `WrongMode`); an accepted RT
  `UseAbility` stores a `StandingOrder` and returns `Ok(None)`. `tick`
  stage 2 runs the clock (+1, `OnTick` delivery, boundary: pending RT-to-TB
  or RT round start) and stage 3 executes due orders through
  `validate_action` and the shared `resolve_use`. `request_mode` only sets
  `pending_mode`; switches apply at an RT boundary or a TB rollover. The
  clock freezes at `RT_CLOCK_MAX`. TB and TB-only worlds are byte-identical
  to before.

## Invariants

1. **Despawn is total, and loading is validated.** A successful `despawn`
   removes the entity, strips every store (the creature record included,
   T139) and the timeline entry, and enqueues exactly one `Despawned`. A dead id returns `false` and enqueues
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
   the only randomness. One recorded exception (T097): the `#[cfg(test)]`
   `thread_local!` `ENCODINGS` counter in `src/history.rs` is a
   per-test-thread count of envelope encodings — a counter, not a thread the
   simulation owns, and test-only. It must stay spelled `thread_local!`;
   `std::thread_local!` would read as a thread to `determinism.py`.
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
14. **The journal's byte total is cached, and `commit` is its only writer
    before validation ends (T097, J1).** The retained journal is a private
    `Journal` holding the envelopes plus a derived canonical byte total. At
    every public boundary that total equals the sum of `envelope_len` over the
    retained envelopes and stays within the byte cap; it is derived, never
    persisted, and `envelope_len` is called exactly once per new envelope —
    never on a retained one — so a mutation costs O(new envelopes + world
    clone). The loader seeds the cache from the total it already computes, and
    acknowledgement subtracts the drained prefix, emptying a fully drained
    journal without encoding. No code outside `commit` assigns the journal,
    the owned world, or `next_seq`, and `commit` writes nothing before every
    check passes.

12. **One legality source (T028, ADR-0018).** Never re-check admission in a
    second predicate or validate by executing on a clone: add a new rule to
    `validate_action` and both execution and enumeration inherit it. Every
    `UseAbility` names a live combatant target whatever `requires_target`
    says; `allow_self_target` alone governs self-targets. Do not cache
    options across state changes; they confer no authorization, and host
    disclosure filtering stays outside sim. Since T137 the last stage of the
    `UseAbility` path is the stat evaluation stage: when the definition has
    derived stats it builds the action plan on a scratch interner and queries
    the defence-side stat, then the actor's bonus stats, and a failure (a zero
    divisor a `Stat` effect or lost health caused) is `StatEvaluation`; it runs
    after affordability and mutates nothing.
13. **Area identity is world-owned and immutable (T027a, ADR-0020).** One
    `World` is one area; presence is liveness, so there is no membership
    index to dangle. Cross-world identity is `(area, EntityId)` — never key
    anything across worlds by `EntityId` alone. Transfer supports only the
    shape it can copy completely: a new per-entity component must extend the
    move contract or make transfer fail, never be silently dropped. A
    creature record moves with its entity, and a transfer into an area that
    already holds a record or combatant for its placement fails
    `AreaError::PlacementInUse` (T139 T1). Viewer
    grants, snapshot cancellation and actionable-mapping retirement belong to
    the host (T027c) and net (T027b), not sim.

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
  stay live, the summary retains the outcome, and no event is emitted. Since
  T139a2 a release also writes each survivor back as a record (below); it
  still despawns nobody. T139b2's `leave_encounter` is a fourth way out: the
  combatant and its entries go through the same settlement as `despawn`, but the
  entity stays live and becomes a record, and neither `Died` nor `Despawned` is
  enqueued.
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

## Kernel range (T137)

The T136b authored surface executes through the kernel: typed and derived
`Int` stats, the `Constant`/`ActorStat`/`Opposed` defences, ability
`bonus_stats`, damage dice, rolled initiative, `Stat` effect targets and tag
grants. No start rejects a T136b shape as unsupported; the T136c bridge
(`UnsupportedAuthored`, its check and its test suite) is deleted, so do not
reintroduce a refusal list: a new shape gets execution or a precise error.

- **Public additions.** `IntFormula`, `DerivedStat`, `InitiativeRule`,
  `COMBAT_BONUS_MOD_TYPE`, `CombatError::StatEvaluation` (`StatEvaluation at
  combat/stats`) and `CombatError::TooManyTags` (`TooManyTags at spec/tags`),
  `AbilityDefense::{Constant, ActorStat, Opposed}`, `EffectTarget::Stat`
  (so `EffectTarget` is not `Copy`), and last fields
  `AbilityDefinition::{bonus_stats, damage_dice}`,
  `CombatDefinition::{derived, initiative}`, `EffectDefinition::grants` and
  `Combatant::tags`, all skipped at their absence value. `definition_is_legacy` and `ability_is_legacy` decide the 15-field
  shape: a new field that must not move a golden is added to them.
- **Only `Int` stats execute.** The kernel has no implicit conversion and
  every combat consumer is an `Int`. The adapter carries no `Fixed`, `Bool`,
  `Enum`, `Tags` or `Dice` stat; a non-derived `Fixed` stat must exist on the
  creature and is never read, and every other non-`Int` or derived stat is
  not read. Do not store a typed value no reader uses (it is hashed state).
  Data owns the formula preconditions (`Int` literals and `Int` names only,
  a buildable kernel pipeline); a breach panics with `derived int stats are
  validated by data read-time checks`.
- **Trap: stat and tag handles come from a per-operation scratch interner.**
  Every operation that reads a stat (`action_plan`, the start preflight, the
  load check) clones `world.interners()` and interns into the clone, in the
  order health, attributes, derived names. Never intern a T137 handle into
  the world: the world interner is hashed, and the `srd_lite` goldens pin it.
  The one write is a `TargetStat` action interning its defence stat, as
  before. A handle from the clone is meaningless outside its operation.
- **Reads are queries, never cached.** A base stat read is a kernel query over
  the combatant's current-health block plus its unexpired `Stat` modifiers, so
  a legacy fight (no `Stat` modifier) reads its stored value through the same
  code. Do not add a second path for legacy content.
- **Bonus stats** are synthetic `Roll` `Add` modifiers of the reserved type
  `combat.bonus_stat`, `StackAll`, ids the smallest unused from `1` in that
  view. An effect using the reserved type is `InvalidEffect` while a listed
  ability carries bonus stats (and a load error).
- **Start order (B21).** Opening checks, placements/areas, per-participant
  creature then stats, initiative rule (dice `InvalidDice`, then bonus names
  `MissingStat`), derived preflight (`InvalidStatValue` naming the derived
  stat), pools, per-ability checks (spec entry, usable attribute, dice, table,
  costs, defence name or `Opposed` dice then bonus names, bonus names, damage
  dice, effect, natural), per-effect checks (empty shape, reserved type, per
  modifier policy checks then `Stat` name), `DuplicateModifier`,
  `PolicyConflict`, `TooManyTags`, publication. A new start check goes in that
  order and mutates nothing; the only draws at start are the initiative keys,
  after the last check.
- **Draw order (T137b).** `Opposed`: the actor's dice, then the opposing dice
  (no natural die on it), on `combat.roll`; the margin is the actor total
  minus the opposing total. Damage dice come after `resolve` and before
  `spend_costs`; an outcome without dice draws nothing. Initiative is rolled
  once at start in authored participant order, in TB and RT alike; the key is
  stored in `Combatant.initiative` (RT timeline keys stay `0`), so rollover
  and the RT-to-TB rebuild read it and draw nothing. Content without these
  shapes must draw exactly as before.
- **Tags.** Every combatant carries its creature's tags, sorted and distinct
  (T139 D5; T139a2 removed T137 B19's "only when a referenced effect grants"
  gate), and a record stores them; grants live on `EffectDefinition.grants`
  and never enter `Combatant.tags`. A view's projection (placement tags plus
  unexpired grants, scratch handles) is the contract a condition consumer
  will read; nothing reads it yet. The union bound is `MAX_TAGS`
  (`TooManyTags`, and the `combatant tags exceed their bound` load error); it
  runs at every start, with an empty grant set when nothing grants.
- **Surface golden (T137c).** `tests/surface_golden.rs` runs the pinned
  seed-42 three-combatant turn-based schedule (`tests/support/surface_schedule.rs`)
  that uses every T137 shape at once, records one `state_hash` per step, and
  compares against the per-target `surface_v1.<target>.txt` baseline after a
  hand-written oracle of the rolled initiative keys, each action's margin and
  each damage total. The authoring example
  (`examples/generate_surface_golden.rs`) shares the schedule source.

## Persistent creature records (T139a1, T139a2, T139b1, T139b2)

- `World.creatures` is the only copy of a creature outside an encounter. An entity holds a `CreatureState` or a `Combatant`, never both; the loader enforces it (K1-K12).
- Public additions: `CreatureState`, `SuspendedEffect`, `CreatureSpec`, `register_creature`, `HistoryWorld::register_creature`, `World::creatures`, `AreaError::PlacementInUse`, and the `CombatError` variants `PlacementInUse`, `RulesetMismatch`, `NoEntrants`. T139a2 adds `Lifecycle`, `EffectsOnExit`, `CombatState.lifecycle`, `Combatant::fixed` and `Combatant::entered`.
- Start's per-prefab stat reading lives in `combat::read_creature_stats` and is shared with registration; change the two together. A registration check order change is a precedence change (R1-R6).
- Trap: a record and a combatant may share a placement only in a respawning encounter (B13): its authored participants spawn fresh and are never written back, while a combatant that *entered* from a record is always written back. Registration (R5) and transfer (T1) refuse to create the other collisions.
- **Lifecycle at start and release (T139a2).** `CombatState.lifecycle` is copied at start from the encounter's `respawns` and the ruleset's `creature_persistence` and `effects_on_exit`, and is skipped at its default. After the definition is built and before the initiative draws, each authored participant resolves to Spawn (respawning, or no record), Skip (a dead record) or Enter (S1 `RulesetMismatch`, S2 `InvalidStatValue`, S3 `TooManyTags`, in that order); no participant left is `NoEntrants`. An entrant takes its health, attributes, pools (unrefreshed), tags and `fixed` from the record, resumes suspended effects as `round + remaining` and has its record removed. `end_encounter` writes back every combatant that entered, and every combatant when the encounter does not respawn, dead included (`health == 0`: the dead stay dead). The write-back is `combat::exit_record`, which T139b's leave shares. `despawn` removes a record with the entity, so a despawned placement spawns fresh at the next start.
- **Members at start (T139b1).** `start_encounter_with_members(world, spec, mode, &[Entrant])` adds record-holding members; `start_encounter_in_mode_tracked` is `start_encounter_with_members_tracked(.., &[])`, and an empty slice must stay byte-identical to `start_encounter_in_mode`. Public additions: `Entrant`, `start_encounter_with_members`, and the `CombatError` variants `NotCreature` and `AlreadyParticipant`. Check order after S1-S3: M0, then M1 and M2 over every member, then M3-M8 per member (`check_ruleset` is S1/M5 and `check_stats_and_tags` is S2-S3/M7-M8, shared with the authored Enter path), then S4. Member keys draw after the participants' draws; members publish after the participants and enqueue nothing.
- **Joining and leaving (T139b2).** Public additions: `join_encounter`, `leave_encounter`, `CombatState.pending_joins`, `Lifecycle.join_after_active` and the `CombatError` variant `EncounterTerminal`. `join_encounter` checks J1-J11 in the contract's order (J9 counts a dead combatant's placement too) and mutates only after the last check, the one `combat.roll` draw being the first mutation. `leave_encounter` checks V1-V4 (a dead combatant cannot leave; it awaits release or despawn) and then goes through `World::remove_combatant_tracked`, which `despawn_tracked` also uses: change the settlement there, once, never in one caller.
- **Journaled join and leave (T139c).** `Joined` and `Left` are version 2 events, so `HistoryEvent::is_v2` must list every v2 variant: forgetting one lets a version-1 wrapper keep it. `HistoryWorld::join_encounter` journals exactly one `Joined`; `leave_encounter` journals `Left` and then `push_transition` of the leave's facts; `start_encounter_with_members` journals what `start_encounter_in_mode` does (members and Enter participants journal nothing). Each stages a clone, runs the free function, drains the legacy queue once and commits.
- Trap: records never upgrade the history version (T139 D12). Do not add a record test to `commit`'s upgrade condition: every TB-only release writes records, and `tests/history_v2.rs` R18 pins that such histories stay at version 1. A version-1 wrapper may hold records.
- Trap: `Lifecycle.join_after_active` is published at every start, including `members == &[]`; it is a ruleset copy, not a member feature.
- Trap: `advance_tracked` begins with the B27 flush of `pending_joins`. Anything that picks a head without `advance_tracked` (a new rollover path) must flush too, or a pending joiner is skipped.
- Trap: a waiting joiner (round 1 or later, or round 0 under the default `late_join`) has no timeline entry and no pending entry, yet it is alive: every alive count includes it, and `tb_round_start` and `switch_to_real_time` are the only things that schedule it. Do not add a third list.
- Trap: the loader's Q1-Q5 compare `(InitiativeKey, EntityId)` with the head, so a pending joiner registered after the head with an equal key is refused. A dead pending joiner is a legal save in a live encounter: the flush drops it. A terminal transition empties `pending_joins` through `combat::set_terminal` (D45a), so a terminal save never holds one; route any new way to end a TB encounter through it.
- Trap: M1 and M2 are a pass of their own so that a bare entity listed twice fails `AlreadyParticipant`, not `NotCreature` (E2's pair). M6's "earlier member" arm cannot be reached from a loadable world, because K10 forbids two records with one placement; it is kept for the contract's wording.
- Trap: add a new start check before S1-S4, never between them; an existing check must keep winning over S1-S4.
- Trap: nothing happens to a creature outside an encounter (D9): no pool refreshes, no effect counts down, and `tick` leaves the store alone.
- Trap: `Combatant.tags` is no longer gated on a granting effect. The hash of any world that starts an encounter over tagged prefabs moved once (the four `crpg-sim` goldens here; four `crpg-testkit` goldens in T139g).
- Tests: `tests/creature_state.rs`, `tests/creature_lifecycle.rs`, `tests/creature_entry.rs` (members and joiner seeding), `tests/creature_join.rs`, `tests/creature_exit.rs`, and `combat::join_leave_facts_tests` (the crate-visible transition facts); the helpers are `support::creature`.

## Movement state and operations (T036a, ADR-0036)

- Public additions (`movement.rs`, re-exported at the crate root): `Mover`, `ActiveRoute`, `MoveRoute`, `NavRevision`, `RouteTag`, `Measure`, `MoveError`, `seg_len`, `point_at`, `route_length`, `route_measure`, `snap`, `to_f32`, `validate_route`, `COORD_LIMIT_RAW`, `MAX_ROUTE_POINTS`, `MAX_ROUTE_LENGTH_RAW`; `World::{movers, nav, bind_nav, place, set_route, clear_route}`; `HistoryWorld::{bind_nav, place, set_route, clear_route}`, `HistoryError::Move`; the `CombatError` variants `InvalidPosition`, `InvalidSpeed`, `InvalidStep`. `movers_mut`, `set_nav` and `HistoryWorld::attach_mover` are crate-private.
- Invariant: movement reads no `f32`; only movement code writes a mover's `Transform`. The mirror is `to_f32` of the position with a velocity (zero outside stage 4); nothing converts an `f32` back, and L6 compares, it does not read.
- Trap: a host write through `transforms_mut` to a mover's transform makes the save fail L6 (`mover transform disagrees with its position`) until the next operation or stage 4 rewrites the mirror. Velocity is not checked.
- Trap: `seg_len` is *horizontal* (`y` ignored), while `point_at` interpolates `y`. A route that only changes height is a `ZeroSegment`.
- Trap: `step` is stored at creation (Reading 2), not read at action time; a later task that lets an operation change a `Fixed` stat must also update the mover.
- Trap: a new creation check (`InvalidPosition`, `InvalidSpeed`, `InvalidStep` and any after them) goes after every existing check: at registration after R6, at start after the lifecycle is built and before the T139 resolution loop. An existing check must keep winning.
- Trap: a mover is built for a *spawned* participant only. Enter participants, members and joiners keep the mover their registration made (the ruleset is checked equal, S1/M5/J8); do not build a second one.
- Trap: `bind_nav` returns the cleared pairs in ascending `EntityId`, not store order (a despawned index is reused, so the two differ). Sort; do not rely on `ComponentStore::iter`.
- Trap: "dead" for `set_route` is a dead combatant *or* a dead creature record (a written-back corpse keeps its mover).
- Trap: a new per-entity movement field must join the mover, the transfer copy (`area.rs`, both functions), the despawn removal and the load rules L1-L12. The history wrappers commit zero events; do not give them an event without a version decision.
- Tests: `tests/movement_core.rs` (the implementer suite; `movement_core_contract.rs` is the two-keys writer's). The MOVE fixture is built from `Fixture::standard()` by JSON edits inside the test file.

## Movement stage 4 (T036b, ADR-0036)

- Public additions: `tick_with_movement(world) -> Vec<MovementFact>`, `HistoryWorld::tick_with_movement() -> Result<Vec<MovementFact>, HistoryError>`, `MovementFact::{Arrived, Cancelled}`, `CancelReason::Dead`. `tick` delegates and discards the facts. `movement_system`, `stop` and `TickFacts.moves` are crate-private.
- Tick order: `run_systems` is now `encounter_clock_system`, `timeline_system`, then `movement_system` (stage 4). Serial territory as before: reordering or adding a stage is a reviewed behaviour change.
- Invariant: stage 4 reads only `Fx16_16` positions and writes the `f32` mirror; it never reads a `Transform`, and it touches no pool, order, timeline, RNG or event. The facts are not history events and the journal does not change.
- Trap: stage 4 visits movers in ascending `EntityId`, not store order; a reused index makes the two differ. Facts must come out in that order too.
- Trap: leftover budget carries across vertices but is discarded at arrival; a budget that lands exactly on a vertex continues with `along == 0` on the next segment.
- Trap: a new way into a turn-based encounter (a start variant, a join variant, a mode switch) must call `stop` on whoever becomes a turn-based combatant, or its routes keep walking under turn-based rules. The three call sites are the turn-based start, `join_encounter` into a turn-based encounter and `switch_to_turn_based`; the switch stops dead combatants too. Do not clear `route` by hand.
- Trap: "dead" for stage 4 is a dead combatant or a dead creature record, as for `set_route`; a corpse's route is dropped with `Cancelled { Dead }`, one fact, not every tick.
- Tests: `tests/movement_tick.rs` (the implementer suite; `movement_tick_contract.rs` is the two-keys writer's) and `tests/movement_golden.rs` with its schedule `tests/support/movement_schedule.rs`. Regenerate the golden with `cargo run -p crpg-sim --example generate_movement_golden --profile test --locked` only with review; the two targets are never compared.

## Movement turn-based move (T036c, ADR-0036)

- Public additions: `TurnMovement { measure, pool, step_cost }` (and `CombatDefinition.movement: Option<TurnMovement>`), `MoveOutcome { actor, cost, from, to }`, `route_cost`, `validate_move`, `perform_move`, `HistoryWorld::perform_move`, `HistoryEvent::Moved { entity, points }`; `HISTORY_VERSION` is now 3. `pool_balance` and `spend_pool` (`combat.rs`) are crate-private.
- Invariant: `validate_move` is the one place a turn-based move is judged and `perform_move` runs it first, then changes state with no other check. The turn checks are `validate_action(EndTurn { actor })` wrapped as `MoveError::Combat`; do not copy them. The cost is `u64` end to end (`CannotAfford` reports the true value); only `route_cost` narrows to `u32`.
- Trap: a new v3 variant joins `HistoryEvent::is_v3`, as a new v2 variant joins `is_v2`; forgetting it lets a version-2 wrapper retain it. `commit` is the only writer of `version`, and it never lowers it.
- Trap: the movement pool is spent from the same place `pool_balance` reads (the primary `action_pool` when it is the primary template, else the matching `extra_pools` entry). A balance read or a spend that looks only at `extra_pools` breaks a ruleset whose movement pool is the ability's pool.
- Trap: `definition_is_legacy` must keep requiring `movement == None`; a legacy-shaped definition that carries movement would otherwise lose it in the 15-field form and reload as `None`.
- Trap: persisting `movement` in the definition moves the `state_hash` of any world started under a ruleset that declares `movement` (the T036b `movement_v1` golden). A new definition field is a re-baseline of every golden whose ruleset sets it; check with the golden tests before assuming "only when `Some`" is enough.
- Trap: the `moved` decoder bounds `points` while streaming (64 points, at least 2, the 65th element never decoded); do not decode into a `Vec` first.
- Tests: `tests/movement_turn.rs` (the implementer suite; `movement_turn_contract.rs` is the two-keys writer's) and `tests/history_v3_golden.rs` with its schedule `tests/support/history_v3_schedule.rs`. Regenerate the golden with `cargo run -p crpg-sim --example generate_history_v3_golden --profile test --locked` only with review; the two targets are never compared.

## Ability range (T149b, ADR-0036 X1-X3)

- Public additions: `within_range(measure, range, from, to)` (`movement.rs`, re-exported), `CombatError::OutOfRange` (fieldless, `OutOfRange at combat/target`) and `AbilityDefinition.range: Option<Fx16_16>`, skipped when `None`. `range_stage` is crate-private.
- Invariant: a new admission stage goes last in `validate_action` unless a contract says otherwise; the range stage is last, so every existing check (`OutOfTurn`, `SelfTarget`, `InsufficientAction`, `DeadTarget`, `StatEvaluation`) keeps winning over `OutOfRange`.
- Invariant: `ability_is_legacy` must keep requiring `range.is_none()`; otherwise the 15-field legacy form drops the range and the reload reads `None`.
- Trap: X3 retains only `Err(OutOfRange)`; a retained order keeps its key and is due every tick (at most once per tick), spending, journaling and re-keying nothing. Any other failure still clears the order and reports `OrderEnded`; do not widen the match arm.
- Trap: a range needs a mover on both sides. A ruleset without `movement` creates no movers, so a ranged ability can never be used there; a mover missing from a save is `OutOfRange`, never "unlimited".
- Tests: `tests/ability_range.rs` (the implementer suite; `ability_range_contract.rs` is the two-keys writer's).

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
- 2026-10-30 (UTC) · claude-code/sonnet-5.5 (implementer) + T139a1 · Added the creature-record contract, its public additions and the shared stat-reading trap; the start/release lifecycle is deferred to T139a2.
- 2026-10-30 (UTC) · claude-code/sonnet-5.5 (implementer) + T139a2 · Recorded the start/release lifecycle (Spawn/Skip/Enter, write-back, `respawns`), amended the Tags and release traps for the removed carriage gate, and added the no-time-outside-an-encounter trap, so a reader does not look for the old gate or an unwritten release.
- 2026-11-07 (UTC) · claude-code/sonnet-5.5 (implementer) + T139b1 · Added the members-at-start contract, its check order and the M1/M2 pass trap to the creature-records section, so the next change to start's checks keeps the order the tests pin.
- 2026-11-07 (UTC) · claude-code/sonnet-5.5 (implementer) + T139b2 · Added the join/leave surface, the pending-flush and waiting-joiner traps and the shared settlement rule, and amended the death/despawn/release trap, so a later change to turn settlement keeps leave and despawn in step.
- 2026-11-07 (UTC) · claude-code/sonnet-5.5 (implementer) + T139b2 · Amended the dead-pending-joiner trap for D45a and named the single terminal helper, so a new terminal path does not leave pending joins that fail Q1 on reload.
- 2026-11-07 (UTC) · claude-code/claude-sonnet-5-5 (implementer) + T139c · Added the journaled join/leave surface and the records-never-upgrade trap to the creature-records section, so a later change to `commit` does not make records a v2 trigger.
- 2026-11-09 (UTC) · claude-code/claude-sonnet-5-5 (implementer) + T036a · Added the movement section: public API, the no-`f32`-read invariant and the traps (host transform writes failing L6, horizontal `seg_len`, stored step, creation-check order, spawned-only movers, id-ordered `bind_nav`, dead records, new per-entity fields).
- 2026-11-09 (UTC) · claude-code/claude-sonnet-5-5 (implementer) + T036b · Added the stage-4 section: the facts reader, the tick order, the three `stop` call sites and the traps (id order, carry-over, new TB entry must call `stop`, dead corpses).
- 2026-11-09 (UTC) · claude-code/claude-sonnet-5-5 (implementer) + T036c · Added the turn-based move section: the public additions, the single `validate_move` judgement, the `is_v3` and persistence traps (including the `movement_v1` hash move), and updated the history version statement to 3.
- 2026-11-10 (UTC) · claude-code/claude-sonnet-5-5 (implementer) + T149b · Added the ability-range section: the new public items, the last-admission-stage invariant, the legacy-form invariant and the X3 and missing-mover traps.
