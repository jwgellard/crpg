# ADR-0013: combat simulation state in `crpg-sim`

Date: 2026-09-26
Status: **Accepted**

## Context

T016 delivers the first end-to-end proof that authored ruleset data drives
combat. T016a (data-owned: four generic combat families, the canonical
`minimal-d6` ruleset, the `combat_basic` campaign) is complete in the working
tree. T016b (sim-owned: data-to-runtime adapter, authoritative combat
components, action/turn/damage controller, persistence and hash coverage)
needs a public-API decision before implementation: the crate contract
requires an ADR for public API changes, and T016 prerequisite 3 requires the
combat API, issuing interner ownership, symbolic persistence, hash
representation, turn policy, event ownership, and compatibility strategy to
be recorded in a reviewed ADR. An unreviewed task sketch is not an ADR; this
file is the reviewed record, approved before any of it is implemented.

## Decision

1. **Combat state lives in `World` as three appended fields, all behavior
   hashed.** `combatants: ComponentStore<Combatant>`,
   `combat: Option<CombatState>`, `interners: Interners`. Every
   behavior-affecting field — health, dead flag, attributes, resource
   balances, round, active turn, timeline order, RNG, the referenced
   immutable combat definition, and the issuing interner — serializes into
   `state_hash` with no exclusions.
2. **Compatibility by absence-skip, never by excluding combat state.** The
   three new fields are serialized only when meaningful:
   `#[serde(default, skip_serializing_if = "ComponentStore::is_empty")]` on
   `combatants`, `#[serde(default, skip_serializing_if = "Option::is_none")]`
   on `combat`, and a private `#[serde(default, skip_serializing_if =
   "interners_empty")]` on `interners`. A non-combat world therefore
   serializes byte-identically to the T007/T008a skeleton and every existing
   non-combat golden is preserved. A combat world serializes its full
   definition and state; no behavior-affecting combat field is ever omitted
   to keep an old hash. Old goldens are not re-baselined.
3. **The issuing interner is world-owned and persisted symbolically.**
   `World` owns one `Interners`; the only runtime handle it interns for
   minimal-d6 is the roll tag. Stat references in combat state remain
   symbolic `String`s (ADR-0006 Decision 4 satisfied — the persisted form is
   the string, never a numeric handle), so no `StatId`-keyed map derives
   serde and the E014 trap stands.
4. **The immutable combat definition is hashed in full, symbolically.**
   `CombatDefinition` holds only serializable symbolic/ULID/integer fields:
   dice as a `DiceExpr` (string form), outcome table as a rules
   `OutcomeTable`, damage as `Vec<(Outcome, u32)>`, pool template as
   `{pool_id: Ulid, pool_max: u32, pool_refresh: RefreshTrigger}`, ability
   `{ability: Ulid, attribute: String, cost: u32, requires_target: bool,
   allow_self_target: bool}`, and content identity `{encounter: Ulid,
   ruleset: Ulid}`. No `StatId`/`TagId`/`StatBlock`/`Modifier`/`QueryContext`
   in `World`.
5. **Turn policy: `Timeline` stays a container.** `end_turn` keeps its
   pop-only semantics. The combat controller owns round structure: at round
   start the timeline holds one entry per live participant at its authored
   `InitiativeKey` (ties broken by `EntityId` ascending); an accepted action
   pops the actor and advances to the next head, rolling the round over when
   the timeline empties. Dead participants are removed from scheduling but
   retain their entity and zero-health terminal state.
6. **Resolution uses `Against::Dc` with the actor's authored attribute, one
   named stream, one interned roll tag, and no modifiers.** `resolve` runs
   over an empty `ModifierPipeline` and transient empty
   `StatBlock`/`TagSet`/`&[Modifier]` views. The actor's attribute value
   (data) is the constant DC; the ability's
   `dice`/`attribute`/`outcome_table`/`damage` (data) drive the rest. Named
   stream constant `combat.roll`; roll-tag symbol `roll` interned into the
   world interner. No hook dispatcher, no effect interpreter, no ruleset
   identifiers in production.
7. **The only new event is `SimEvent::Died { entity }`, core-closed.** It is
   produced exactly once on the positive-health-to-zero transition and
   consumed by the terminal-state assertions (tests, then T016c) and future
   AI/effect systems. No rules `KernelHook` is emitted or dispatched, so no
   correlation ULID is required in this task; hook payloads remain
   core-closed and this task adds none.

## Consequences

- `World`'s serialized shape grows by three defaulted fields; deserialization
  additionally rejects combatant ids that are not live, `combat = Some` with
  an empty combatant store, `combat = None` with a non-empty combatant
  store, and an `active` id that is not a live combatant.
- Non-combat worlds keep their exact bytes and hashes; combat worlds hash
  every behavior-affecting field, including resources, turn state, queued
  events, RNG, and the combat definition.
- The sim gains runtime `[dependencies]` on `crpg-data` and `crpg-rules`
  (path edges, manifest-level approval recorded in `tasks/T016b.md`);
  both are consumed read-only. No other crate changes.
- T016c replays through `start_encounter` / `perform_action` only; no
  private world access path is added for it.

## Agent log

- 2026-09-26 (UTC) · opencode/muse-spark + T016b approvals · Filed as the reviewed ADR for the combat public API, world-owned symbolic state, absence-skip compatibility, container-preserving turn policy, kernel resolution wiring, and Died-only event ownership, after maintainer approval of the T016b dependency edges and this decision text.
