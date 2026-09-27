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
[ADR-0013](../adr/0013-combat-simulation-state.md) and
[ADR-0014](../adr/0014-combat-encounter-release.md).
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
