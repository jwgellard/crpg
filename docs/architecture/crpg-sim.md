# crpg-sim — architecture

One loaded area's simulation state: the authoritative world the server owns.

**State:** the skeleton is complete (T007): `World` with spawn/despawn/query,
`ComponentStore<T>`, the `Timeline` container, `Transform`, `SimEvent`, and
the live event queue. Systems, the tick loop, `state_hash`, movement and
every further component are planned, each with its owning task named in the
module docs.

Decisions: [ADR-0006](../adr/0006-crpg-core-primitives.md),
[ADR-0007](../adr/0007-reserved-arena-generation.md) and
[ADR-0008](../adr/0008-event-ownership.md).
Working contract: [`crates/crpg-sim/AGENTS.md`](../../crates/crpg-sim/AGENTS.md).

---

## Position

`crpg-sim` sits above `crpg-core`, `crpg-data` and `crpg-rules`: it may reach
all three, and everything live — `crpg-net`, `crpg-script`, `crpg-persist`,
the server — reaches it. Today it uses only the `crpg-core` edge; the
`crpg-data` and `crpg-rules` edges are for T010-era schema types and T014-era
stats, not for speculative imports.

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
  store, timeline, event queue, RNG and tick. Spawn/despawn/query only;
  nothing advances. `EntityMeta` is a reserved placeholder (braced, for the
  serde reason the module doc states).
- **`store` — `ComponentStore<T>`.** One dense `IndexMap`-backed store per
  component type. Dumb by design: no liveness checks, insertion-order
  iteration, pair-list serde. The no-dangling invariant is `World`'s job.
- **`timeline` — `Timeline`, `InitiativeKey`.** The container only:
  `BTreeMap<(key, id)>` with replace-on-insert and O(n) removal. Advance
  policy is T008's.
- **`transform` — `Transform`.** `f32` position and velocity, the only
  floating point in the crate (E006-A). No rotation until movement needs it.
- **`event` — `SimEvent`.** `Spawned` / `Despawned` only. Grows as systems
  arrive, one variant per consumer-backed vocabulary decision.

## Today versus planned

| Exists (T007) | Planned (owner) |
|---|---|
| Skeleton, spawn/despawn/query, skeleton serde | Tick loop, systems, `state_hash` (T008) |
| `Timeline` container | Advance policy (T008) |
| `SimEvent` spawn/despawn | Further variants with their systems |
| `Transform` position/velocity | Rotation, movement, collision (later) |
| Reserved `SimDelta` seam (E015) | Full delta shape, `apply_delta` (T018) |
| — | `StatBlock` components (T014), spatial index, dynamic store |

## What consumers inherit

- **Identity semantics** from the arena, unchanged: generations from 1,
  lowest-index reuse, ascending iteration, `u32::MAX` tombstone. Every save
  and every golden hash downstream inherits them.
- **The closed event contract**: envelopes ordered `(tick, seq)`, queue
  bytes part of the hashed world (T008 covers them in `state_hash` goldens
  per ADR-0008).
- **The `f32`-spatial / `Fx16_16`-rules split.** Anything a consumer computes
  a rule from must cross into integers or fixed point before `crpg-rules`
  sees it. The boundary is enforced at this crate's edge by the `no-f64`
  lint, not by goodwill.

## Agent log

- 2026-09-06 (UTC) · opencode/muse-spark + T007 · Wrote the crate doc for the skeleton: position above core/data/rules, module map, today-vs-planned table with owners, and what consumers inherit.
