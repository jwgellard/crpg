# ADR-0108 (draft): Kernel range, time model, and ruleset-described player UI

Date: 2026-10-17 (UTC)
Status: **Proposed (draft by opencode-go/beam-free)**

## Context

T108 is the architecture package gated by D28l and recorded at
`docs/IMPLEMENTATION_PLAN.md:206`. It must settle three things before
T034 (movement ADR), T104 (player UX) and T109 (a third, unlike-family
ruleset) can proceed: (1) whether the seven rules primitives in `crpg-rules`
cover the CRPG system families; (2) how real-time-with-pause and turn-based
combat coexist in one game; (3) how a ruleset describes its player UI so
that any ruleset displays correctly. It is set against a blind draft from
another lab; this draft makes its own investigation.

The current kernel is the T014/T015 surface in `crpg-rules` (architecture:
`docs/architecture/crpg-rules.md:15-57`). Its modules are `stats`, `derived`,
`modifier`, `dice`, `resolution`, `resource`, `hooks`, `error`; the public
surface is re-exported at the crate root (`crates/crpg-rules/src/lib.rs:13-20`).
The seven primitives are defined at `docs/CRPG_ENGINE_SPEC.md:329-337`:
Stat, Modifier, Effect, Resolution, Resource, Tag, Event/Hook. The two
existing rulesets are `rulesets/minimal-d6/` and `rulesets/srd-lite/`.

The platform and determinism invariants are non-negotiable
(`AGENTS.md`, non-negotiable section): no floats in `crpg-core`/`crpg-rules`,
`f32` only for spatial positions in `crpg-sim`, no `HashMap`/`HashSet`,
exact-build replay with independently generated target-scoped goldens. The
S-set (`docs/IMPLEMENTATION_PLAN.md:86-98`) serializes one agent at a time
for changes to `World` or tick order, schema versions and migrations in
`crpg-data`, `crpg-contracts`, and the GDExtension boundary.

## Decision 1 — Kernel range

### What the kernel supports today

- Stats: `StatValue` is `Int(i32) | Fixed(Fx16_16) | Bool | Enum(EnumId) |
  Dice(DiceExpr) | Tags(TagSet)` (`docs/CRPG_ENGINE_SPEC.md:343-348`;
  `crates/crpg-rules/src/stats.rs`). `StatBlock` is an insertion-ordered
  `IndexMap` with standalone validation.
- Modifiers: `ModOp` is `Add | Multiply | Set | Clamp`; stacking policies
  are `StackAll`, `HighestBonusWorstPenalty`, `HighestPriorityPerName`
  (`docs/CRPG_ENGINE_SPEC.md:352-363`; `crates/crpg-rules/src/modifier.rs`).
  Modifiers carry `SourceRef`, `ModifierTarget` (`Stat | Roll(RollTag) | Dc`),
  `ConditionExpr`, `mod_type`, `priority`.
- Resolution: one generic `ResolutionRequest` with `RollSpec`, `Against`
  (`Dc | Stat | Opposed`), `OutcomeTable`, `Outcome` including `Custom(u8)`,
  and `NaturalRule`/`NaturalEffect` (`Shift | Override`)
  (`docs/CRPG_ENGINE_SPEC.md:365-380`; `crates/crpg-rules/src/resolution.rs`;
  `docs/architecture/crpg-rules.md:108-116`).
- Resources: `ResourcePool { id, max, current, refresh }` with
  `RefreshTrigger = OnTurnStart | OnRoundStart | OnRest(RestId) | OnTick(n) |
  Never` (`docs/CRPG_ENGINE_SPEC.md:382`; `crates/crpg-rules/src/resource.rs:36-77`).
- Tags: interned strings on entities, items, actions, damage, effects
  (`docs/CRPG_ENGINE_SPEC.md:336`).
- Hooks: nine variants — `OnDeath`, `OnTurnStart`, `OnTurnEnd`,
  `OnRoundStart`, `OnEncounterStart`, `BeforeRoll`, `AfterRoll`,
  `BeforeDamage`, `AfterDamage` (`crates/crpg-rules/src/hooks.rs:26-108`).
- Dice: one group plus optional integer offset, with keep/drop selection
  (`crates/crpg-rules/src/dice.rs:1-104`).
- Effects: **not** a kernel type. `crpg-rules` holds no `Effect` struct,
  store, or lifecycle (`docs/architecture/crpg-rules.md:12-13`). An effect is
  a data-level bundle of modifiers and hook handlers applied and removed by
  the host.

### Family: d20 (D&D 5e / PF2e-shaped)

| Primitive | Supported now | Gap (smallest kernel change) | Out of scope |
|---|---|---|---|
| Stat | Int/Fixed/Bool/Enum/Dice/Tags (`stats.rs`) | None | — |
| Modifier | Add/Multiply/Set/Clamp; stacking policies match 5e (same-name) and PF2e (highest bonus/worst penalty) (`modifier.rs`) | None | — |
| Effect | Modifiers + tags; condition value via `Int` | Duration/lifecycle tracking not in kernel; host (sim/data) owns apply/remove (`docs/architecture/crpg-rules.md:12-13`) | Effect store in kernel |
| Resolution | Outcome tables with natural-20 shift/override; opposed/DC/stat; margin `i64` (`resolution.rs`) | None for d20 bands | — |
| Resource | `action`, `bonus_action`, `reaction`, `movement` as separate pools; costs per ability (`resource.rs`) | None | — |
| Tag | Tags on entities/items/actions/damage/effects | None | — |
| Event/Hook | 9 variants (`hooks.rs:26-108`) | Spec lists `OnApplyEffect`, `OnRemoveEffect`, `OnMove`, `OnActionDeclared` not yet present (`docs/CRPG_ENGINE_SPEC.md:388`) | Hook dispatcher/registry (host owns) |

### Family: dice pool (e.g. World of Darkness, Shadowrun)

| Primitive | Supported now | Gap (smallest kernel change) | Out of scope |
|---|---|---|---|
| Stat | Int counts and thresholds | None | — |
| Modifier | Add to pools/DCs | None | — |
| Effect | As above | As above | Effect store |
| Resolution | Rolls a `DiceExpr` and compares margin | `DiceExpr` sums kept faces; no success-counting against a threshold (`dice.rs:1-104`) | Exploding dice, rerolls, graduated successes unless T109 proves need |
| Resource | Pools refresh on tick/turn | None | — |
| Tag | Tags | None | — |
| Event/Hook | As above | As above | — |

Smallest change that closes the dice-pool gap: extend `DiceExpr` with an
optional success threshold and a count-success evaluation mode (or a
parallel `DicePoolExpr`), still one group, still drawing from a named
stream, still traceable. This touches `crpg-rules` dice, serde, and the
literal-exclusion gate (`crates/crpg-rules/AGENTS.md`, limits section).

### Family: percentile (d100)

| Primitive | Supported now | Gap (smallest kernel change) | Out of scope |
|---|---|---|---|
| Stat | Int; `DiceExpr` sides up to `MAX_DIE_SIDES` = 1,000,000 (`crates/crpg-rules/AGENTS.md`) | None | — |
| Modifier | Add to roll/DC | None | — |
| Effect | As above | As above | — |
| Resolution | Outcome tables can define percentile bands; roll-under via `Against::Dc/Stat` | None | Two-dice (tens/ones) presentation is client-side |
| Resource | Pools | None | — |
| Tag | Tags | None | — |
| Event/Hook | As above | Missing variants (see d20) | — |

### Family: action points

| Primitive | Supported now | Gap (smallest kernel change) | Out of scope |
|---|---|---|---|
| Stat | Int | None | — |
| Modifier | Add to pools/DCs | None | — |
| Effect | As above | As above | — |
| Resolution | Generic | None | — |
| Resource | `ResourcePool` with `OnTick(n)` cooldowns and `OnTurnStart` refresh; atomic per-pool affordability in sim (`resource.rs`; `docs/architecture/crpg-sim.md:169-182`) | None | — |
| Tag | Tags | None | — |
| Event/Hook | As above | Missing variants | — |

### Family: classless skill-based

| Primitive | Supported now | Gap (smallest kernel change) | Out of scope |
|---|---|---|---|
| Stat | Skills as Int stats; derived expressions (`stats.rs`, `derived.rs`) | None | — |
| Modifier | Circumstance/status modifiers | None | — |
| Effect | As above | As above | — |
| Resolution | Outcome tables; opposed checks | None | — |
| Resource | Pools for fatigue/mana | None | — |
| Tag | Tags for skill categories | None | — |
| Event/Hook | As above | Missing variants | Progression/XP/levels are ruleset data, not kernel (`docs/CRPG_ENGINE_SPEC.md:405`) |

### Options considered for Decision 1

- **Option A — Freeze the kernel at seven primitives and close only the
  missing hook variants.** Keeps the abstraction test honest; dice-pool
  counting is deferred to T109.
- **Option B — Extend `DiceExpr` now with success counting and add an
  `Effect` struct to the kernel.** Richer, but pre-empts T109 and risks
  building a system-specific shape into the kernel.
- **Option C — Add a general "roll result transform" hook so rulesets
  implement counting in Lua.** Keeps kernel minimal, but pushes a hot path
  into script and hurts determinism/auditability.

### Recommendation (Decision 1)

Adopt **Option A**: keep the kernel at seven primitives, add the four
missing hook variants (`OnApplyEffect`, `OnRemoveEffect`, `OnMove`,
`OnActionDeclared`) to `KernelHook` in `crates/crpg-rules/src/hooks.rs`
(core-closed fields only, per ADR-0008/ADR-0011), and defer dice-pool
success counting to T109. If T109 cannot express a dice-pool ruleset with
existing primitives, the smallest acceptable change is the `DiceExpr`
threshold extension in Option B's first half, filed as a new A task, not
silently added. Do not add an `Effect` type to the kernel; effects stay a
data bundle applied by the host, consistent with
`docs/architecture/crpg-rules.md:12-13` and spec §3.4.

S-set / invariant impact: the hook-variant addition is confined to
`crpg-rules` and does not touch `World`, tick order, schema versions,
`crpg-contracts`, or the GDExtension boundary. It respects the no-float,
no-`HashMap`, exact-build invariants. The deferred dice change, if ever
taken, is also `crpg-rules`-only but touches serde/persistence and must
re-run the literal-exclusion gate.

## Decision 2 — Time model (RTwP and turn-based in one game)

### Current engine facts

- The container is `Timeline` in `crates/crpg-sim/src/timeline.rs` (module
  documented at `docs/architecture/crpg-sim.md:56-59`): a
  `BTreeMap<(InitiativeKey, EntityId)>` with replace-on-insert and O(n)
  removal. Advance policy lives in `tick`/`end_turn`
  (`docs/architecture/crpg-sim.md:67`).
- Spec §2.5 (`docs/CRPG_ENGINE_SPEC.md:312-318`): server tick 20 Hz fixed;
  durations in rounds/turns/ticks; both modes are one `Timeline`, advancing
  every tick in real time and on `EndTurn` in turn-based; key type
  `(InitiativeKey(i32), EntityId)`.
- `World` owns the timeline, RNG, tick, and event queue
  (`docs/CRPG_ENGINE_SPEC.md:284-299`).
- Combat is world-owned encounter state with `start_encounter` /
  `perform_action` / `end_encounter` (`docs/architecture/crpg-sim.md:68-82`).
- Area identity and transfer exist (`docs/architecture/crpg-sim.md:223-238`).

### Prior art (R14)

R14 (`docs/research/T099/R14-deep.md`) recommends: host-granted, revocable
pause permission with split pause/unpause rights (rec 1); per-encounter
on-the-fly RTwP↔TB switching, never locked per campaign (rec 2); late-join
initiative rules — round 1 keeps a full turn slotted behind acted
combatants, later joins skip, same-player tied characters act
simultaneously (rec 3); auto-pause as a server-side policy with per-trigger
toggles, host-controlled, default-off in multiplayer (rec 4); turn timers,
delay/skip, shared visible order, party AI disabled in TB (rec 5); split
parties — only the engaged group enters TB, the rest stay real-time and
join on approach/attack (rec 6); trivial dialogue/stores do not pause,
important dialogue pauses with a party vote and host tiebreak (rec 7); large
dedicated worlds get no global player pause, DM-only (rec 9); never
hard-eliminate a player — spectatorship (rec 11).

### Options

- **Option A — Global mode per `World`/area.** A single `mode: TimelineMode`
  flag on `World`; the tick loop advances the timeline every tick only in
  RTwP mode and only calls systems otherwise. Simplest; matches the
  container's current single-policy shape.
- **Option B — Per-encounter mode.** `CombatState` carries its own
  `TimelineMode`; the world timeline stays real-time for exploration, and
  an encounter's mode governs its participants. Split parties are natural
  because only the engaged group's encounter is TB. This is what R14 rec 6
  requires.
- **Option C — Per-entity mode tag.** Each entity carries a mode; the tick
  loop advances only RTwP entities each tick and holds TB entities. Most
  flexible, but mixes policies inside one queue and makes initiative
  ordering and determinism harder to reason about.

### Recommendation (Decision 2)

Adopt **Option B**: mode is per-encounter on the existing `CombatState`, not
per-`World` or per-entity. Exploration stays real-time on the world
timeline; an encounter flips its participants into TB; split parties fall
out naturally because only the engaged group's encounter is TB, and the rest
of the party stays on the world timeline until they approach or attack
(R14 rec 6). Mode switching is a policy flip on `CombatState`, not a second
engine, and is allowed on the fly per encounter (R14 rec 2), never locked
per campaign.

Pause: pause is a host-granted, revocable permission with split pause and
unpause rights (R14 rec 1). It is **not** a `World` mutation and not part
of `state_hash`; the host stops advancing the affected world/encounter while
paused, and pause/unpause commands are host inputs recorded by the net
layer for replay. Auto-pause is a server-side policy with per-trigger
toggles, host-controlled, default-off in multiplayer (R14 rec 4). Trivial
dialogue and stores never pause; important dialogue pauses with a party
vote and host tiebreak (R14 rec 7). Large dedicated worlds have no global
player pause; pause is DM-only there (R14 rec 9). Late-join initiative
follows R14 rec 3 (round 1 full turn slotted behind acted combatants; later
joins skip; same-player tied characters act simultaneously). Turn timers,
delay/skip, and a shared visible order are part of TB (R14 rec 5).
Spectatorship, not elimination, for sidelined players (R14 rec 11).

S-set / invariant impact: **yes.** Making the real-time policy advance the
`Timeline` inside `tick()` is a tick-order change
(`docs/architecture/crpg-sim.md:67`; spec §2.5) and is S-set; it requires
H-review and new goldens before implementation. This draft still recommends
it because spec §2.5 already commits to one mechanism with two policies, but
it must not be implemented without human approval. Per-encounter mode on
`CombatState` is sim-local and does not by itself change `World` or tick
order. Pause is kept out of `state_hash` and out of `HistoryWorld`, so no
history-schema or persistence change is required.

What snapshots and the protocol must carry:
- `HistoryWorld` already carries `TurnStarted` and `EncounterEnded` as
  branch-observed facts (`docs/architecture/crpg-sim.md:183-204`;
  ADR-0017). Add the encounter's `TimelineMode` and the current round/turn
  holder to the snapshot's encounter block.
- Initiative order (the `Timeline` entries) must be in the snapshot so a
  client or late joiner can render the shared visible order (R14 rec 5).
- Late-join state: round number and whether the joiner is slotted or
  skipped (R14 rec 3).
- Mode and pause state are host-controlled and travel on the host control
  path, not in the sim snapshot; the client renders from both. Projection
  follows ADR-0019: host supplies per-field optional values, suppresses an
  event unless every field is permitted, preserves order.
- Area transfer already persists area identity
  (`docs/architecture/crpg-sim.md:223-238`); mode travels with the
  encounter, not the area.

## Decision 3 — Ruleset-described player UI

### Current engine facts

- Character creation is already a ruleset-defined wizard schema interpreted
  by the client UI; the kernel has no character generator
  (`docs/CRPG_ENGINE_SPEC.md:403-406`).
- Presentation is a client-side mirror; UI reads the replica world through a
  stable query API (`docs/CRPG_ENGINE_SPEC.md:881-882`).
- The client receives a filtered subset including presentation assets and UI
  definitions (`docs/CRPG_ENGINE_SPEC.md:565`).
- T051 (HUD, combat log, `SimEvent`-driven VFX/audio) waits on T108 and
  T104; T104 will refine this later (`docs/IMPLEMENTATION_PLAN.md:243`,
  `docs/IMPLEMENTATION_PLAN.md:252`).
- R13: Foundry separates `system` data/schema from sheets and generates UI
  from schema; sheets are views (`docs/research/T099/R13-deep.md:5,33`).
- R15: asset validation is a hard gate at import (rig conformance, 16-char
  IDs, texture sets) (`docs/research/T099/R15-deep.md:1-9`).

### Options

- **Option A — Ruleset ships full UI definitions (layouts, widgets, code).**
  Maximum flexibility; conflicts with the T2 rule that client-side code never
  executes (`docs/CRPG_ENGINE_SPEC.md:1079`) and with generated forms
  (`docs/CRPG_ENGINE_SPEC.md:1051`).
- **Option B — Engine ships fixed templates; ruleset supplies only names and
  values.** Safe and consistent, but cannot express a ruleset-specific sheet
  (e.g. PF2e's three-action economy, a skill tree) and pushes every new
  system into engine code.
- **Option C — Hybrid: ruleset declares a declarative UI descriptor
  (screens, sections, field bindings, display hints, asset references);
  client renders through fixed component types.** Data only, no code; matches
  R13's schema-first discipline and T104's later refinement.

### Recommendation (Decision 3)

Adopt **Option C**. A ruleset provides a `ui` block (schema version bumped
in `crpg-data`, S-set) describing, at data level only:

- Screens: `character_sheet`, `action_bar`, `spellbook`, `inventory`,
  `level_up`, `combat_hud`, plus optional `custom` screens with a stable id.
- Sections and fields, each binding to a stat, ability, resource, tag, or
  computed derived stat by id, with a display kind
  (`number`, `percent`, `dice`, `modifier_breakdown`, `tag_set`, `icon`),
  label key, icon asset id, and visibility condition expressed as a tag or
  simple stat comparison — never code.
- Action bar entries derived from abilities filtered by tag, with cost
  display from resource pools.
- Spellbook grouping by tag/school and resource cost.
- Inventory slots and item stat bindings.
- Level-up screen driven by the ruleset's progression table
  (`docs/CRPG_ENGINE_SPEC.md:405`).
- Combat HUD: health/resource bars, initiative order, action economy
  display, all bound by id.
- Asset references validated at import per R15 (rig, 16-char ids, texture
  sets).

The client never executes ruleset code (T2, `docs/CRPG_ENGINE_SPEC.md:1079`);
it renders descriptors through fixed components and reads values via the
replica query API (`docs/CRPG_ENGINE_SPEC.md:881`). T104 refines layout and
interaction; this ADR fixes only the data the ruleset must provide.

S-set / invariant impact: the `ui` block lands in the ruleset schema, so it
touches schema versions and migrations in `crpg-data` (S-set) and the
presentation-manifest filter (T072). It does not touch `World`, tick order,
`crpg-contracts` signatures (presentation manifest already carries UI
definitions per `docs/CRPG_ENGINE_SPEC.md:565`), or the GDExtension
boundary. No float or `HashMap` is introduced; descriptors are serialized
JSON.

## Consequences

- T034 (movement ADR) may proceed once this ADR is accepted: movement
  rules are mode-aware but the mode container is already the per-encounter
  `CombatState`, so no new timeline primitive is needed.
- T109 (third ruleset, unlike family) becomes the gate for the dice-pool
  gap: if it cannot be expressed, file a `crpg-rules` A task for the
  `DiceExpr` threshold extension; do not patch the kernel inside T109.
- T104 (player UX research) and T051 (HUD) inherit the `ui` descriptor
  contract; T051's acceptance criteria should check that a ruleset with no
  `ui` block still renders through engine defaults.
- T110 (rules editor, after M6) gains a UI-descriptor editor as a
  schema-generated form (`docs/CRPG_ENGINE_SPEC.md:1051`).
- Pause and mode are host-controlled inputs, not sim state, which keeps
  `state_hash` and `history_hash` stable but moves replay responsibility to
  the net layer; this must be documented in T024/T025a.
- The hook-variant addition is the only immediate `crpg-rules` change and
  must ship with its doctests and the hooks suite.

## Open questions for the user

1. **Should the real-time policy advance the `Timeline` inside `tick()` (S-set)?**
   Recommendation: yes, because spec §2.5 already commits to one mechanism
   with two policies, but only with H-review and new goldens; otherwise keep
   RTwP as host-driven `end_turn`-every-tick (simpler, less deterministic
   for the dry-run simulator).
2. **Should pause state be recorded as a `HistoryEvent` variant?**
   Recommendation: no — keep pause as a host input outside `HistoryWorld`,
   so `history_hash` and the journal stay unchanged; revisit if replay of
   paused sessions is required.
3. **Dice-pool counting: extend `DiceExpr` or use Lua?**
   Recommendation: defer to T109; if needed, extend `DiceExpr` with a
   threshold mode (kernel change, `crpg-rules`-only) rather than Lua.
4. **Should a ruleset be allowed to override the engine's default screens
   entirely, or only extend them?**
   Recommendation: extend-only (engine defaults always present), matching
   T2's no-code rule and R13's schema-first discipline; full override
   requires a later ADR.
5. **Auto-pause default in single-player.**
   Recommendation: on (BG-style), off by default in multiplayer
   (R14 rec 4), host-toggleable.
6. **Late-join initiative: BG3 rule (round 1 full turn, later skip) or
   always-skip?**
   Recommendation: adopt R14 rec 3 as written.

## S-set and invariant summary

- Touches S-set: Decision 2's tick-loop policy (tick order) — yes, pending
  H-review; Decision 3's `ui` block (schema versions/migrations in
  `crpg-data`) — yes; Decision 1's hook variants — no.
- Does not touch: `crpg-contracts` signatures, the GDExtension boundary.
- Invariants: all recommendations are compatible with no floats in
  `crpg-core`/`crpg-rules`, `f32`-only-spatial in `crpg-sim`, no
  `HashMap`/`HashSet`, and exact-build replay. None requires weakening a
  test or changing a golden without H-golden.

## Agent log

- 2026-10-17 (UTC) · opencode-go/beam-free + T108 draft · Drafted the ADR covering kernel range across five system families, the per-encounter time model with host-controlled pause, and the declarative ruleset UI descriptor, with S-set and invariant impact stated.