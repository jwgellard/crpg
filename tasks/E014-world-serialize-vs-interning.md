## Task
Reconcile "the entire `World` implements `Serialize`" with interned handles
having no serde. Human-decision task (spec edit).

## Why this is deferred

- The spec defines `StatBlock` as `IndexMap<StatId, StatValue>` with no
  conversion layer (`docs/CRPG_ENGINE_SPEC.md:280`) and asserts "the entire
  `World` implements `Serialize`/`Deserialize`" (`:243-244`).
- ADR-0006 Decision 4 deliberately gives `StatId`/`TagId` no `Serialize`/
  `Deserialize` — persisted form is always the string, via explicit
  `to_serializable(&Interner)` / `from_serializable(&mut Interner)`
  conversion, landing in T014 (`docs/adr/0006-crpg-core-primitives.md:160-190`:
  "there is no stat data in the `World` until the rules kernel exists").
- A derived `World: Serialize` containing a `StatId`-keyed map cannot hold
  literally. T007's save/load round-trip and T014's conversion pair will be
  built against opposite assumptions unless the spec carries the caveat.

## Decision to make
- Confirm the conversion-pair design (derived serde for the world skeleton,
  explicit string conversion at every interned-handle boundary); state which
  side owns each half and where the round-trip property test lives.

## Deliverable
- Edits to `docs/CRPG_ENGINE_SPEC.md` (§§2.4, 3.2) with dated inline notes.
  No source changes.

## Constraints
- Doc-only. Human sign-off; must agree with ADR-0006 D4, not amend it.

## Agent log

- 2026-09-06 (UTC) · opencode/muse-spark · Filed as part of the spec-gap triage: derived-World-serde and handle-string-persistence contradict as written.
- 2026-09-06 (UTC) · opencode/muse-spark + maintainer sign-off · Confirmed the conversion-pair design (no ADR-0006 change): derived serde covers the T007 skeleton only, T014 owns the string pair plus its round-trip test, T007 builds no `StatBlock`; spec §§2.4/3.2/24 carry the caveat.
