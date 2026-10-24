# crpg-sim — architecture

One loaded area's simulation state: the authoritative world the server owns.

**State:** the skeleton (T007), the loop and the instrument (T008a), and
authoritative headless combat (T016b): `Combatant` / `CombatState` /
`CombatDefinition` world-owned encounter state, the `start_encounter` /
`perform_action` controller over the timeline container, and the `Died`
vocabulary. Systems beyond the timeline step and combat, movement and
further components are planned, each with its owning task named in the
module docs.

Decisions: [ADR-0006](../adr/0006-crpg-core-primitives.md),
[ADR-0007](../adr/0007-reserved-arena-generation.md),
[ADR-0008](../adr/0008-event-ownership.md),
[ADR-0009](../adr/0009-determinism-scope.md),
[ADR-0013](../adr/0013-combat-simulation-state.md),
[ADR-0014](../adr/0014-combat-encounter-release.md),
[ADR-0016](../adr/0016-sim-multi-ability-pools-effects-turns.md) and
[ADR-0017](../adr/0017-opt-in-authoritative-history.md).
Working contract: [`crates/crpg-sim/AGENTS.md`](../../crates/crpg-sim/AGENTS.md).

---

## Position

`crpg-sim` sits above `crpg-core`, `crpg-data` and `crpg-rules`: it may reach
all three, and everything live — `crpg-net`, `crpg-script`, `crpg-persist`,
the server — reaches it. Since T016b it uses all three edges: data for the
authored combat vocabulary it adapts read-only, rules for the resolution
kernel it calls read-only.

Two structural facts follow from the position. First, **the tick order that
will live here is serialise-one-agent-at-a-time territory** (spec §15.2):
changing tick order changes behaviour globally, so T008's loop gets one
author at a time and human review. Second, the crate is the meeting point of
three vocabularies that must not collapse into one: the generic event
substrate (core), the campaign content types (data) and the rules kernel
(rules). ADR-0008's split — mechanism down, vocabulary up — is what keeps
`World` from becoming the crate everything is stuffed into.

## Modules

- **`world` — `World`, `EntityMeta`.** The skeleton: entity arena plus one
  store, timeline, event queue, RNG and tick. Spawn/despawn/query plus the
  T008a advance; `EntityMeta` is a reserved placeholder (braced, for the
  serde reason the module doc states). Deserialization validates the
  no-dangling shape before a `World` is built, extended by the T016e combat
  coherence check (roll tag, definition declarations, health/dead/maximum
  agreement, pool/attribute/placement identity, active-head scheduling);
  despawning the last combatant releases the encounter while retaining
  interners, events, RNG, and tick.
- **`store` — `ComponentStore<T>`.** One dense `IndexMap`-backed store per
  component type. Dumb by design: no liveness checks, insertion-order
  iteration, pair-list serde. The no-dangling invariant is `World`'s job.
- **`timeline` — `Timeline`, `InitiativeKey`.** The container only:
  `BTreeMap<(key, id)>` with replace-on-insert and O(n) removal. Advance
  policy is T008a's (`tick` preserves standing order; `end_turn` pops).
  Loading rejects duplicate entities; `From<Vec<_>>` keeps the last entry.
- **`transform` — `Transform`.** `f32` position and velocity, the only
  floating point in the crate (E006-A). No rotation until movement needs it.
- **`event` — `SimEvent`.** `Spawned` / `Despawned` plus combat's `Died`
  (core-closed `entity`, emitted exactly once on the positive-to-zero
  health transition). Grows as systems arrive, one variant per
  consumer-backed vocabulary decision.
- **`tick` — `tick`, `end_turn`, `run_systems`.** The fixed-step loop and
  both advance primitives. Order is spec §10, handwritten; today one system.
- **`combat` — `Combatant`, `CombatState`, `CombatDefinition`,
  `start_encounter`, `perform_action`, `end_encounter`.** The authoritative encounter
  controller. The data-to-runtime adapter converts validated authored
  records (checked whole-number fixed-to-int stats, parsed dice, built
  outcome tables, cost/pool conversion) all-or-nothing; the controller
  resolves through the rules kernel over the world-owned RNG stream,
  spends costs, applies saturating nonnegative damage, emits `Died` once,
  and advances turns (next head, round rollover, terminal) while the
  timeline stays a container and `end_turn` stays pop-only. `end_encounter`
  releases any active encounter with a retained per-participant summary:
  scheduling and components stripped, entities live, no events emitted.
  Despawn of the active combatant advances or terminates through the same
  shared rule; second encounters reuse content with fresh runtime ids. Stat
  references persist as symbolic strings; the issuing interner is
  world-owned and persists as its ordered string list.
- **`hash` — `state_hash`.** BLAKE3 over canonical JSON, queue bytes
  included, exclusions none (governed list per ADR-0009, starting empty).
  Non-finite floats are rejected up front because JSON would collapse them
  to `null`.
- **`history` — `HistoryWorld`, `HistoryEvent`, `history_hash`.** The opt-in
  authoritative journal (T020, ADR-0017): one privately owned `World` plus
  its bounded single-consumer journal of adjacent-tagged history envelopes.
  Typed transactional mutations stage a full clone, reuse the combat
  controller on staged state, drain and translate the staged legacy queue
  exactly once, attach branch-observed turn/terminal facts, validate the
  journal in pinned order, and publish only on success — gameplay errors
  first, rejection committing nothing. Reads page at most 256 envelopes
  with stale/future cursors and inert semantics; acknowledgements retire
  the contiguous prefix monotonically and idempotently. `history_hash`
  covers the complete canonical wrapper with no exclusions; the legacy
  path — `SimEvent`, free functions, bytes, hashes — is untouched.

## Today versus planned

| Exists (T007–T008a) | Planned (owner) |
|---|---|
| Skeleton, spawn/despawn/query, skeleton serde | Systems beyond the timeline step (their tasks) |
| Tick loop, `Timeline` advance, `state_hash` (T008a) | Harness + goldens (T008b), replay (T009) |
| `SimEvent` spawn/despawn | Further variants with their systems |
| Combat controller, encounter state, `Died` (T016b) | Multi-ability encounters, effect interpreter (T017+) |
| `Transform` position/velocity | Rotation, movement, collision (later) |
| Reserved `SimDelta` seam (E015) | Full delta shape, `apply_delta` (T018) |
| — | `StatBlock` components (T014), spatial index, dynamic store |

## What consumers inherit

- **Identity semantics** from the arena, unchanged: generations from 1,
  lowest-index reuse, ascending iteration, `u32::MAX` tombstone. Every save
  and every golden hash downstream inherits them.
- **The closed event contract**: envelopes ordered `(tick, seq)`, queue
  bytes part of the hashed world (covered in `state_hash` goldens per
  ADR-0008).
- **The `f32`-spatial / `Fx16_16`-rules split.** Anything a consumer computes
  a rule from must cross into integers or fixed point before `crpg-rules`
  sees it. The boundary is enforced at this crate's edge by the `no-f64`
  lint, not by goodwill.

## Compatibility with B1 shapes (T017b)

The adapter reads the primary authored pool from `Ruleset.pools.first()`
into the unchanged single-pool runtime; fixtures carry legacy-equivalent
values. Action, turn, damage, RNG, persistence, and release semantics are
unchanged. The temporary supported subset (single pool, first-ability
legacy shapes) and its pre-mutation rejection of plural pools, extra costs,
non-ending turns, effects, target-stat defense, and natural selection are
specified in `tasks/T017b.md`; the rejection error itself waits on its
narrow ADR and is not implemented here. Full generalization stays in B3.

As-built (primary part): adapter uses `pools.first()` with legacy fixtures
and two new regressions; sim suites stay green with legacy goldens
byte-identical; boundary plus downstream CLI-migrate consumer update remain
open (see `tasks/T017b.md`).

As-built (boundary, ADR-0015 Accepted): single `UnsupportedAuthored`
gate with per-field Display, pinned validation/field order, first-only
support, and multifault/non-first regressions; 40 combat tests green on
Windows/MSVC with legacy goldens identical; per-field retirement stays
with B3.

## Generalization to multi-ability/pool/effect/turn (T017d, specified before source)

The controller generalizes to per-ability ULID definitions, plural pools
with atomic per-pool costs, target-stat/actor-attribute defense selection
with transient stat views, explicit natural-die selection, lifetime-bearing
attached effects rebuilt from the hashed definition on every query with
effect-scoped policies, and explicit turn completion (`ends_turn: false`
skips the advance; `EndTurn` advances with no spend/draw/damage/effect).
The definition embeds ability/pool/effect vectors with absence-skip hash
preservation (legacy single-ability/pool fields retained from the
first-listed entry; new vectors persisted together only for non-legacy
shapes); the `UnsupportedAuthored` gate retires field-by-field as each
shape gains execution. Decisions: [ADR-0016](../adr/0016-sim-multi-ability-pools-effects-turns.md).
Full execution stays in B3; replay/goldens stay in B4.

As-built: the controller, definition vectors with joint absence-skip
persistence, extended load coherence, deleted gate with six-site
`combat_multi` execution coverage, and unchanged legacy goldens per target;
the breaking API required a user-authorized narrow mechanical fix in the two
downstream adapters (effects wiring plus one `Option` unwrap, no
golden/behavior change); B4 owns replay/goldens, B5 the CLI proof.

## Kernel range (T137, ADR-0029 Follow-on 4)

The adapter executes the T136b authored surface through the `crpg-rules`
kernel. T137a carries typed and derived `Int` stats, the `Constant` and
`ActorStat` defences alongside `ActorAttribute` and `TargetStat`, ability
`bonus_stats`, and `Stat` effect targets; T137b adds the `Opposed` defence,
damage dice, rolled initiative and tag grants. No start rejects a T136b shape
as unsupported: the T136c bridge (`CombatError::UnsupportedAuthored`, its
check and `tests/unsupported_authored.rs`) is deleted.

**Public additions.** `IntFormula`, `DerivedStat` and `InitiativeRule` (the
symbolic sim-owned mirrors; persisted state never holds a `crpg_data` wire
type); `COMBAT_BONUS_MOD_TYPE`; `CombatError::{StatEvaluation, TooManyTags}`;
new last variants `AbilityDefense::{Constant, ActorStat, Opposed}` and
`EffectTarget::Stat` (`EffectTarget` is no longer `Copy`); new last fields
`AbilityDefinition::{bonus_stats, damage_dice}`,
`CombatDefinition::{derived, initiative}`, `EffectDefinition::grants` and
`Combatant::tags`. Every new field is skipped at its
absence value, and `definition_is_legacy` keeps the 15-field definition for
every legacy shape, so no existing hash or golden moves.

**Only `Int` stats execute.** The kernel has no implicit conversion and every
combat consumer (health, attributes, defences, bonus stats, `Stat` targets)
is an `Int`, so no other kind can reach a margin, a key or a fold. A
non-derived `Fixed` stat must be present on a creature and is never read;
`Bool`, `Enum`, `Tags` and `Dice` stats and every derived stat of another
kind are not read at all. `CombatDefinition.attribute_names` is the base
`Int` stats, `derived` the derived `Int` stats in authored order.

**Stat reads.** Every read is one kernel query per operation over the `Int`
definitions (health, each attribute, each derived formula), never cached, so
a `Stat` effect or lost health changes the next read. A combatant's block is
its current health plus each attribute. Each view holds the combatant's
unexpired modifiers: the actor's `Roll` and `Stat` (and `Dc` for
`ActorStat`), the target's `Dc` and `Stat`, one combined view when
self-targeted. Bonus stats become synthetic `Roll` `Add` modifiers under the
reserved type `combat.bonus_stat` (policy `StackAll`, ids the smallest unused
from `1`), and an effect using that type is refused whenever a listed
ability carries bonus stats.

**Start order (B21).** `EncounterActive`, `ModeUnsupported`, collection
bounds and the data-owned asserts; placements, then `MixedArea`, then
`AreaMismatch`; per participant `MissingCreature` then each declared stat
(`MissingStat`, `InvalidStatValue`, `ValueOverflow`); the initiative rule's
dice (`InvalidDice`) and bonus names (`MissingStat`); the derived-stat
preflight (`InvalidStatValue` naming the first derived stat that
fails to evaluate); pool templates; per listed ability the spec entry, the
checked attribute (must be usable), dice, outcome table, costs, defence name
(for `Opposed` its dice, then its bonus names), bonus names, damage dice,
effect, natural die; per referenced effect (ascending) the
empty-shape check, the reserved bonus type, then per modifier the policy
checks and the `Stat` target name; `DuplicateModifier`; `PolicyConflict`;
`TooManyTags`; publication, whose first draws are the rolled initiative keys.
Nothing before publication mutates the world.

**Mid-fight evaluation.** A `Stat` modifier or lost health can zero a divisor
after a clean start, so `validate_action` gains a last stage, after
affordability and only for a definition with derived stats: it builds the
action's plan and queries the defence-side stat, then the actor's bonus
stats, then an `Opposed` defence's bonus stats on the target. A failure is
`StatEvaluation` naming that stat; execution and `legal_actions` inherit it.

**Draws.** An `Opposed` defence resolves as an opposed total: the actor's
dice draw first and the opposing dice second, on `combat.roll`, with no
natural die on the opposing roll. The target's view carries its `Roll`
effects and the opposing bonus (and its `Dc` effects, which no fold reads);
a self-targeted use has one view where the actor's bonus list is followed by
the opposing list, so both rolls fold both. Damage dice are drawn after the
resolution and before the cost is spent: the first flat `damage` entry for the
decided outcome (or `0`) plus the outcome's dice, summed in `i64` and clamped
once into `0..=u32::MAX`; an outcome without dice draws nothing. A ruleset
initiative rule is rolled once at start, in authored participant order after
every check and before the spawn loop, in both modes: the key is the dice
total plus the participants' bonus stats plus the authored key, saturating,
stored in `Combatant.initiative`. A TB start and every TB rollover insert it,
an RT start keys `0` and keeps it, and an RT-to-TB switch rebuilds at it.
Content without these shapes draws exactly as before.

**Tags.** Creature tags are carried (sorted, distinct) in `Combatant.tags`,
and each effect's `grants`, only in an encounter whose referenced effects
grant, so every other world keeps its bytes. A view's tag projection is its
combatant's tags plus the grants of its unexpired attachments, interned into
the operation's scratch interner. Nothing reads a tag yet (rebuilt modifiers
carry no condition); the projection is the contract a later condition
consumer reads. The union of one participant's tags and every grant is
bounded by the kernel's 1024: over it is `TooManyTags` at start and a load
error.

**Load coherence.** `World` deserialization tests the usable names (base plus
derived) where it tested attributes, and adds the derived-stat, kernel,
bonus-name (ability, `Opposed` and initiative), `Stat`-target, reserved-type
and combatant tag-bound checks.

**Surface golden (T137c).** `tests/surface_golden.rs` runs the pinned seed-42
three-combatant turn-based schedule (`tests/support/surface_schedule.rs`) that
uses every T137 shape at once, records one `state_hash` per step, and
compares against the per-target `surface_v1.<target>.txt` baseline after a
hand-written oracle of the rolled initiative keys, each action's margin and
each damage total. The authoring example
(`examples/generate_surface_golden.rs`) shares the schedule source.

## Atomic same-pool affordability (T019)

The T018c adjacent defect (same-pool entries passing per-amount validation
then panicking in `spend_costs`) is repaired as ADR-0016 §2's atomic-cost
promise: `perform_action` sums every pool's entries as one `u32` total in
authored template order, primary first, with checked addition. An
unrepresentable sum returns `ValueOverflow`; a representable total above
balance returns `InsufficientAction` with the total and original balance; the
first failing pool in template order wins. Only fully affordable actions reach
resolution/spending (exact sums; zero entries harmless); persisted vectors keep
their order, RNG draw order is unchanged, adapter per-entry validation and load
coherence are preserved, and existing successful actions stay byte-identical
per target. Covered by the 11-test `tests/combat_costs.rs` public-API suite.

## Opt-in authoritative history (T020)

`HistoryWorld` wraps one privately owned `World` with a bounded journal of
`HistoryEvent` envelopes (`Spawned`, `Despawned`, `Died` mirroring the
drained legacy queue; `ActionResolved` with symbolic outcome text and
reported damage; `TurnStarted` and `EncounterEnded` from branch-observed
controller facts, never inferred). Each typed operation validates through
the existing controllers on staged state and publishes only after the
journal validates in pinned order — strings, sequence range, event count
(4096), canonical byte total (one MiB) — with gameplay errors first and
atomic rollback otherwise. Retention is the contiguous suffix
`(acknowledged, next_seq)` with checked non-wrapping sequences, 256-envelope
pages, monotonic idempotent acknowledgement, and a single host consumer
(T022). Persistence keeps `version`, `world`, `acknowledged`, `next_seq`,
`pending` with streaming type-before-value payloads and bounded visitors;
retained-history bounds are distinguished from JSON parser scratch, whose
mandatory pre-parse byte cap is the host loading boundary. `history_hash`
(BLAKE3 over the complete canonical wrapper) has no exclusions. Legacy
`state_hash`, emissions, bytes, and goldens are unchanged per target, and
the new `history_v1` goldens are generated independently per native target
from the pinned sim-local schedule with its hand-authored oracle.

### Journal byte accounting and world-only staging (T097)

The retained journal is a private `Journal`: the envelopes plus their derived
canonical byte total, held to the envelopes at every public boundary (J1) and
never persisted, so wrapper bytes, hashes and goldens are unchanged. Each
mutation stages `self.world.clone()` instead of a wrapper clone and ends in
one `commit`, which validates the additions against the live wrapper in the
pinned order — strings, sequence range, event count, byte total — and then
publishes world, journal and sequence together or not at all. The byte stage
starts from the cached total and encodes only the new envelopes, so a
mutation costs O(new envelopes + world clone) and a zero-event operation
encodes nothing; acknowledgement subtracts the drained prefix, and a fully
drained journal is emptied without encoding. The loader hands the total it
already computes to `Journal::from_loaded` rather than re-encoding. A
`#[cfg(test)]` `thread_local!` `ENCODINGS` counter makes that cost a test
count, never a wall clock.

## Shared read-only combat legality (T028)

`combat` exposes the pure admission check as `validate_action(&World,
&CombatAction)` and a bounded enumerator `legal_actions(&World, EntityId)`
(ADR-0018). `perform_action` runs `validate_action` first, so execution and
queries share one validation path with the pinned multifault precedence and
T019's per-pool sums; neither query interns, creates or draws RNG streams,
emits events, spends, or advances. `legal_actions` validates `EndTurn` for the
actor first (its failure is `LegalActionsError::Actor`), then keeps the
`UseAbility` candidates the validator accepts over abilities in ascending
ULID order and current combatants in ascending full `EntityId` order, with
`EndTurn` last. Output is bounded by `MAX_LEGAL_ACTIONS = 4096` including
`EndTurn`, failing wholly (`TooManyOptions`) rather than truncating; the bound
is on output size, not running time. Options are not leases: host,
network entitlement filtering and AI consumption belong to higher crates,
and execution always revalidates. No persisted field, hash, replay or golden
changes.

## Area identity and explicit transfer (T027a)

`World` gains an optional, immutable authored area ULID (ADR-0020), set only
by `World::new_in_area` / `HistoryWorld::new_in_area`; unbound worlds omit it
and keep legacy bytes, bound worlds persist and hash it in full. The fact is
deliberately minimal: presence in the area is liveness in the world
(`area_of`), so there is no per-entity membership index; death and release
keep presence. A bound world's `start_encounter` rejects participants whose
resolved area differs (`AreaMismatch`). The new `area` module holds the only
producer that moves an entity between worlds: `transfer_entity` for plain
worlds and `transfer_history_entity` for history wrappers, which stage both
authorities, despawn in the source and spawn a fresh identity (with the copied
optional transform) in the destination, and publish both or neither. Only
noncombat, unscheduled entities move. Runtime identity across worlds is
`(area, EntityId)`. Consumers: T027b projects whole-area presence; T027c
validates the authored area against loaded content and owns viewer grants.

## Encounter mode and real-time policy (T138a, ADR-0029)

One encounter is turn-based (TB) or real-time (RT), chosen by the host and never by the sim.

- **State shapes.** `CombatState` gains `mode: EncounterMode` (omitted when TB), `clock: Option<EncounterClock { round_ticks, elapsed }>` (`None` means TB-only) and `pending_mode: Option<EncounterMode>`. `Combatant` gains `order: Option<StandingOrder { ability, target, repeat }>`. `AbilityDefinition` gains `recovery_ticks: Option<u32>` and `repeats_in_real_time: bool`. All are absent in TB-only worlds, so their bytes, hashes and goldens do not move; `ability_is_legacy` also requires both RT fields to be empty so a save of an RT-capable single-ability encounter keeps its vectors. `RT_CLOCK_MAX` is `i32::MAX`.
- **Public API added.** `EncounterMode`, `EncounterClock`, `StandingOrder`, `RT_CLOCK_MAX`, `start_encounter_in_mode`, `request_mode`, `CombatAction::Stop`, `CombatError::{ModeUnsupported, WrongMode}`. `start_encounter` still publishes TB.
- **Admission.** `validate_action` stays the one legality source. In RT only its turn stage is replaced: `UseAbility` and `Stop` meet a terminal gate (`OutOfTurn { active: None }` when fewer than two combatants are alive), `EndTurn` fails `WrongMode`, and `Stop` in TB fails `WrongMode`. An accepted RT `UseAbility` stores the order and returns `Ok(None)`.
- **Tick order.** Stage 2 (`encounter_clock_system`) advances the RT clock by one, delivers `OnTick` refreshes, and at a boundary (`elapsed % round_ticks == 0`) applies a pending RT-to-TB switch or runs the RT round start. Stage 3 (`timeline_system`) then snapshots the entries with key `<= elapsed` and executes each stored order through `validate_action` and the shared `resolve_use`; an invalid due order is cleared, a surviving actor is re-keyed at `elapsed + recovery`, a one-shot order clears. The tick that reaches `RT_CLOCK_MAX` does nothing else and the encounter then freezes. TB and TB-only ticks are unchanged.
- **Switches.** A mode request only sets `pending_mode`. RT to TB applies at an RT boundary (orders cleared, TB round start). TB to RT applies at a TB rollover (everyone keyed at the clock, RT round start). A TB rollover of an RT-capable encounter advances the clock by `round_ticks`, saturating, and delivers `OnTick` refreshes.
- **Load coherence.** `World::deserialize` rejects incoherent RT saves (rules L1-L5 of `tasks/T138.md`) so the stages never meet them at tick time.
- **History.** History v2 (T138b) journals these transitions; see the next section.

## History v2 (T138b, ADR-0029 N5)

`HISTORY_VERSION` is 2 and version 1 is still read. A new wrapper starts at 1 and upgrades lazily and for good: the first successful `commit` whose staged world holds `combat.clock: Some(_)`, or whose events include a v2 variant, sets `version` to 2 (`HistoryWorld::version()` reads it). `commit` stays the only writer of the journal, world, `next_seq` and `version`; a rejected commit changes none of them, so every TB-only history keeps its bytes, hash and `history_v1` goldens.

- **Events.** `HistoryEvent` gains `RoundStarted { round }`, `ModeChanged { mode, round }` and `OrderEnded { actor }`, adjacent-tagged like the rest (`round_started`, `mode_changed`, `order_ended`) with manual duplicate/unknown/missing-key visitors; `OrderEnded` has its own `actor` visitor, not `EntityRef`.
- **Facts.** `TransitionFacts` gains `mode_changed` and `round_started`. `tick_tracked(world) -> TickFacts` returns the stage-2 transition, the stage-3 `TickEvent`s (`Resolved(ActionOutcome)`, `OrderEnded { actor }`) in execution order, and the terminal edge; `tick` is `let _ = tick_tracked(world);`, so its bytes do not change. `World::despawn_tracked` captures the mode and non-dead count before the removal so an RT despawn that drops the count from two or more to below two reports `encounter_ended`.
- **Emission order.** `tick`: `ModeChanged`, `RoundStarted`, `TurnStarted`; then each `ActionResolved` followed at once by its `Died`, and each `OrderEnded`; last one `EncounterEnded`. `perform_action` and `despawn` keep today's order with the trailing transition extended to `ModeChanged`, `RoundStarted`, then `TurnStarted` or `EncounterEnded`. RT order setting, `Stop` and `request_mode` emit nothing; an RT start journals only its `Spawned` events.
- **Operations.** `HistoryWorld::start_encounter_in_mode` and `request_mode` stage, run the combat function and commit; `request_mode` commits zero events and so needs no free slot or sequence. `HistoryWorld::tick` no longer asserts that it emits nothing; a tick whose events would pass a cap returns the history error and commits no clock advance and no RNG draw.
- **Loader.** `version` must be 1 or 2 (`unknown history version`). Payloads decode with both vocabularies whatever the version; after all keys are read a version-1 wrapper is rejected if a retained payload is a v2 variant (`history v1 journal holds a v2 event`) or if its world holds `combat.clock: Some` (`history v1 wrapper holds a real-time-capable encounter`).
- **Goldens.** `tests/goldens/history_v2.<target>.txt` is generated independently per native target by `examples/generate_history_v2_golden.rs` from `tests/support/history_v2_schedule.rs`, and checked by `tests/history_v2_golden.rs` after a hand-written event oracle.

## Agent log

- 2026-09-06 (UTC) · opencode/muse-spark + T007 · Wrote the crate doc for the skeleton: position above core/data/rules, module map, today-vs-planned table with owners, and what consumers inherit.
- 2026-09-06 (UTC) · opencode/muse-spark + T008a · Moved the loop, advance policy and hash into Exists; recorded the governed-empty exclusion list.
- 2026-09-06 (UTC) · opencode/muse-spark + sim invariant hardening · Recorded validated World/Timeline loading and the explicit non-finite hash guard.
- 2026-09-26 (UTC) · opencode/muse-spark + T016b crate opening · Moved the combat controller, world-owned encounter state with symbolic persistence, and the Died vocabulary into Exists; recorded the live data/rules edges and ADR-0013.
- 2026-09-27 (UTC) · opencode/muse-spark + T016e invariant repair · Recorded the load-time combat coherence check and the despawn-last release semantic in the world module.
- 2026-09-27 (UTC) · opencode/muse-spark + T016f crate opening · Recorded the release operation with retained summary, the despawn-active advance rule, and content-reusing reentry in the combat module with ADR-0014.
- 2026-09-27 (UTC) · opencode/muse-spark + T017b crate opening · Recorded the primary-pool adapter rule with legacy-equivalent fixtures and held the support boundary behind its narrow ADR before source implementation.
- 2026-09-27 (UTC) · opencode/muse-spark + T017b primary implementation · Aligned the module with the as-built pools.first adapter and regressions; boundary and consumer updates remain open per tasks/T017b.md.
- 2026-09-27 (UTC) · opencode/muse-spark + T017b boundary implementation · Aligned the module with the accepted per-field gate, pinned orders, and multifault coverage; retirement stays with B3.
- 2026-09-27 (UTC) · opencode/muse-spark + T017d crate opening · Recorded the accepted multi-ability/pool/effect/turn generalization with absence-skip persistence and per-field retirement under ADR-0016 before source implementation.
- 2026-09-27 (UTC) · opencode/muse-spark + T017d implementation · Aligned the module with the as-built generalization, joint absence-skip persistence, extended coherence, deleted gate with combat_multi coverage, unchanged legacy goldens, and the user-authorized downstream mechanical fix.
- 2026-09-28 (UTC) · opencode/muse-spark + T019 implementation · Recorded the atomic same-pool affordability repair (template-order sums, checked overflow, first-failing wins) with its combat_costs coverage and unchanged per-target goldens.
- 2026-09-29 (UTC) · opencode/muse-spark + T020 implementation · Recorded the opt-in history wrapper with its transactional journal, bounded read/ack, full-wrapper hash, retained-vs-parser-memory persistence boundary, and independently generated per-target history goldens; legacy path unchanged.
- 2026-09-30 (UTC) · claude-code + T028 implementation · Recorded the shared validate_action/legal_actions surface (single validation path, pinned enumeration order, whole-failure output bound) so consumers see a read-only query with no persisted or hash change.
- 2026-09-30 (UTC) · claude-code + T027a implementation · Recorded the persisted area identity, liveness-based presence and the atomic two-world transfer producer, naming the net/host consumers that own disclosure.
- 2026-10-17 (UTC) · opencode-go/deepseek-v4.1-flash + T097 implementation · Recorded the cached journal byte total (J1) and world-only staging through `commit`, which keep every mutation O(new envelopes + world clone) with unchanged bytes, hashes, and goldens; the test-only `ENCODINGS` counter is named so the cost guard stays a count.
- 2026-10-23 (UTC) · opencode-go/deepseek-v4.1-flash + T136c · Recorded the temporary kernel-range bridge: `UnsupportedAuthored` refuses every T136b shape before placement with an unchanged world, and T137 deletes the variant, check, and suite.
- 2026-10-23 (UTC) · claude-code (Sonnet 5.5) + T138a · Recorded the per-encounter mode, RT clock, standing orders, mode requests and the two RT tick stages with their load checks, so readers know TB-only worlds are byte-identical and where history v2 (T138b) will attach.
- 2026-10-24 (UTC) · claude-code (Sonnet 5.5) + T138b · Recorded history v2: the lazy sticky version upgrade, the three new events and facts, the tick emission order, the v1 loader rejections and the v2 golden, so readers know TB-only histories still write version 1.
- 2026-10-24 (UTC) · claude-code (Sonnet 5.5) + T137a · Replaced the bridge section with the kernel-range section: what executes now, the `Int`-only rule, the per-operation stat reads, the start order and the new mid-fight evaluation stage, so readers know T137b still owns `Opposed`, damage dice, initiative and tags.
- 2026-10-24 (UTC) · claude-code (Sonnet 5.5) + T137b · Removed the remaining bridge text and recorded the `Opposed` draw order, damage dice, rolled initiative and tag carriage with the `TooManyTags` bound, so the kernel-range section describes the whole executed surface.
- 2026-10-24 (UTC) · opencode-go/beam-free + T137c · Added the surface golden paragraph and the pinned seed-42 schedule/example/test/baseline, so the generalized surface has a per-target golden with a hand-written value oracle.
