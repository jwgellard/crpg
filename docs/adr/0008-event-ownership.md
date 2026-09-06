# ADR-0008: event ownership — substrate in core, vocabularies above

Date: 2026-09-05
Status: **Accepted**

## Context

E001 found the project inconsistent about where events live: role labels put
"events" in `crpg-core` (spec §2.2, README workspace table), spec §16.2 promises
core "event ordering" property tests, but T006a–e built no event types and the
core architecture doc declares the primitives complete — while the spec's
`World` sketch owns `events: EventQueue` in `crpg-sim`.

Consumer analysis (full trace in the E001 research record) shows "events" are
three vocabularies with different homes, and the dependency table decides:

- `crpg-net`, `crpg-script`, `crpg-persist`, `crpg-godot`, `crpg-cli` can all
  reach `crpg-sim`. Only `crpg-edit` (`{core,data,rules}`) and
  `crpg-contracts` (`{core}`) cannot.
- `crpg-rules` can reach only `{core,data}` — and `sim` already depends on
  `rules`, so `rules -> sim` would be both an upward edge and a cycle.

## Decision

Mechanism down, vocabulary up. Core establishes that events *exist*; downstream
crates say what they *are*:

1. **Core holds a generic, game-agnostic event substrate only** —
   `EventEnvelope<P>` / `EventQueue<P>` with `(Tick, seq)` ascending-drain
   contract, canonical serialization, and property-tested ordering. Payload `P`
   is bounded to core-closed field types (`EntityId`, `Tick`, integers /
   `Fx16_16`, `Ulid`, `String` — never `StatId`/`TagId` handles per ADR-0006
   Decision 4, never `rules`/`sim` types). No game variants, no dispatch, no
   routing, no handlers. This is the `GenerationalArena<T>` shape, not a trait:
   a concrete generic container, static dispatch, no `dyn`.
2. **Concrete `SimEvent` enum and the live queue live in `crpg-sim`** —
   `World.events: EventQueue<SimEvent>`. Every live-event consumer reaches sim.
3. **Event-IR graph types (`Trigger` / `Node` / action signatures) live in
   `crpg-data`** as campaign content. The editor, script compiler, persistence,
   and schema tooling all reach data.
4. **Kernel hook types and handler signatures live in `crpg-rules`**
   (`BeforeRoll`, `OnDeath`, …). Only rules and sim need them, and sim already
   depends on rules.
5. **No new traits; no change to `crpg-contracts`.** Contracts is human-owned;
   nothing here needs a workspace-wide trait.

Task assignment: substrate rides with **T007** (see exception below);
IR types with **T010**; hooks with **T014**; `SimEvent` + queue instance with
**T007**.

One-task-one-crate exception, stated per `AGENTS.md`: the substrate is core
code built during a sim task. Justification: it is ~60 lines, fully generic,
with no domain content, and its only consumer in-tree is T007's `World`; a
separate single-purpose task costs more process than protection. If the
substrate grows beyond the generic container, that growth is its own task.

## Consequences

- Core's closed Public API list (`crates/crpg-core/AGENTS.md`) gains two or
  three names — an ADR-level change, which this is.
- Queue bytes become part of the hashed world; T007/T008 must cover queue
  ordering and round-trip in `state_hash` golden tests.
- `crpg-rules` handlers take hook types defined in rules itself; sim dispatches
  them. No cycle, no table change.
- The editor validates graphs against `crpg-data` types. No `ALLOWED` amendment
  anywhere in the workspace.
- Spec §16.2's "event ordering" line now means the generic queue's drain order,
  tested in core; game-payload ordering is covered in sim.

## Rejected

- **Game enum in core** (`SimEvent { Damage, Dialogue, Quest, … }` in
  `crpg-core`): puts combat/dialogue/quest vocabulary in the primitives crate,
  contradicting its finished-scope declarations and subjecting game churn to
  the strictest stability contract in the workspace (cf. `DiceExpr` excluded
  from core in ADR-0006).
- **Both in sim**: strands the editor's graph validation — `crpg-edit` cannot
  reach `crpg-sim` — forcing a layering exception for purely structural types.
- **Trait-based** (`EventPayload` trait in core or contracts): no precedent in
  core (zero traits workspace-wide); contracts is human-owned and out of scope
  for an agent-executed decision; a trait still leaves the queue logic to be
  written per crate instead of once.

## Agent log

- 2026-09-05 (UTC) · opencode/muse-spark + E001 decision · Recorded the A′ split decided with the maintainer: generic substrate in core, SimEvent in sim, IR types in data, hooks in rules.
