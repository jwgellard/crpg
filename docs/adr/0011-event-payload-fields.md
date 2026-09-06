# ADR-0011: event payload bound — fields, not the enum

Date: 2026-09-06
Status: **Accepted**

Supersedes the wording (not the shape) of ADR-0008 Decision 1. ADR-0008
itself is immutable and stays as written; where the two disagree, this ADR
governs.

## Context

ADR-0008 Decision 1 bounds the core event payload `P` to "core-closed field
types (`EntityId`, `Tick`, integers / `Fx16_16`, `Ulid`, `String` — never
`StatId`/`TagId` handles per ADR-0006 Decision 4, never `rules`/`sim`
types)." Read over `P` itself, "`never sim types`" forbids
`EventQueue<SimEvent>` — which Decision 2 of the same ADR mandates
(`World.events: EventQueue<SimEvent>`), and which `crpg-core/src/event.rs`
module docs call "a layering violation, full stop." The code follows
Decision 2, so the text disagrees with itself and with the tree.

The evidence that the bound was meant over payload *fields*: `SimEvent`
(`crpg-sim/src/event.rs`) has exactly two variants, `Spawned` and
`Despawned`, each carrying a single `EntityId` — every field core-closed.
The design copies the established `GenerationalArena<T>` pattern (generic
container in core, instantiated with a downstream value type such as
`EntityMeta` in sim). Nothing about the shape needs changing; only the
sentence describing the bound is wrong.

## Decision

1. **The core-closed bound applies to the field types composing a payload,
   not to the vocabulary enum itself.** Every field inside a payload must be
   a core-closed type (`EntityId`, `Tick`, integers / `Fx16_16`, `Ulid`,
   `String`; never `StatId`/`TagId` handles, never component data, never
   `rules`/`sim` types as nested field types). The vocabulary enum itself
   lives in the crate that owns it: `SimEvent` in `crpg-sim`, event-IR graph
   types in `crpg-data` (T010), kernel hook types in `crpg-rules` (T014).
2. **`World.events: EventQueue<SimEvent>` is the sanctioned instance**, not
   an exception. It complies with Decision 1 as clarified here: the enum
   lives in sim, all its fields are `EntityId`.
3. **Violations, unchanged in spirit:** defining game vocabulary inside
   `crpg-core`; putting non-core-closed fields in any payload; persisting
   interned handles. Enforcement stays review-based — `P` remains an
   unconstrained generic, since a compiler bound would take a trait and
   ADR-0008 forbids new traits.
4. **No code changes.** The tree already implements this reading; this ADR
   aligns the words with the tree. The `event.rs` module docs, the
   `crpg-core/AGENTS.md` trap entry, and `docs/architecture/crpg-core.md`
   are updated to cite this ADR instead of the old sentence.

## Consequences

- Reviewers cite this ADR rather than flagging `World.events`; T010/T014
  inherit the clarified rule for IR and hook payloads.
- The "layering violation, full stop" sentence in `event.rs` is narrowed to
  its intended meaning (vocabulary in core, or unclean fields anywhere).

## Rejected

- **Editing ADR-0008 in place:** ADRs are immutable; a contradictory sentence
  is superseded, never rewritten.
- **Strict-over-`P` enforcement** (treat `EventQueue<SimEvent>` as a genuine
  violation): forces moving `SimEvent` into core or forking a sim-local
  queue — both rejected by ADR-0008 with reasons that still hold (game churn
  under the strictest stability contract; stranding `crpg-edit`'s graph
  validation).

## Agent log

- 2026-09-06 (UTC) · opencode/muse-spark + ADR-0008 clarification · Filed per maintainer choice of the fields-reading option: bound over payload fields, sanctioned sim instance affirmed, module/contract/arch docs aligned with no code changes.
