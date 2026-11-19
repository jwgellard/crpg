# T052-draft-b — persisted continuation state (`crpg-sim`)

## Task
T052 adds persisted continuation state to `crpg-sim`'s `World` (D27f,
`tasks/DECISIONS-2026-10-04.md:87`): the data over which the T053 event-IR
interpreter and the T054 dialogue engine and quest state machine will run
(`docs/IMPLEMENTATION_PLAN.md:283`–`284`). When this is done, a script graph
paused in `Wait(ticks)` (`docs/CRPG_ENGINE_SPEC.md:617`), an open dialogue
(`docs/CRPG_ENGINE_SPEC.md:642`) and a quest's current stage
(`docs/CRPG_ENGINE_SPEC.md:239`–`260`, authored shape) all live in the
`World`, survive save/load and server restart, and resume exactly
(`docs/CRPG_ENGINE_SPEC.md:624`–`625`); the save/load equivalence gate still
holds (`docs/CRPG_ENGINE_SPEC.md:853`). `crpg-sim` stays free of
`crpg-script`: the continuation is world-owned serializable IR state, and a
live Lua stack/coroutine is never saved (`docs/CRPG_ENGINE_SPEC.md:624`;
`crates/crpg-script/src/lib.rs:10`; `tasks/POST-T018-DECISIONS.md:381`). One
part, in `crpg-sim` only.

Citation convention (T153's): a bare `:N` or `:N–M` is a line of the file
cited earlier in the same item or sentence. Every citation was checked on
this branch.

## Parts
| Part | Crate | Scope | Size (source + tests) | S-set | Review |
|---|---|---|---|---|---|
| T052 | `crpg-sim` | continuation/dialogue/quest state: three stores, tick stage 5, save version | ~120 + ~200 suite | yes (`World` touch, `docs/IMPLEMENTATION_PLAN.md:282`) | Claude reviewer mutant run on `src/continuation.rs`, one fix round |

Order: one part. T053 (`crpg-script` interpreter,
`docs/IMPLEMENTATION_PLAN.md:283`) and T054 (dialogue engine and quest state
machine, `:284`) build on it and need no rebase discipline with it.

### T052 owns
- `crates/crpg-sim/src/continuation.rs` (new: the three component types,
  the stage-5 countdown, the due query)
- `crates/crpg-sim/src/world.rs` (three last fields, `WorldRepr` entries,
  load validation, `despawn_tracked`, transfer preflight, getters)
- `crates/crpg-sim/src/tick.rs` (one stage-5 call in `run_systems`)
- `crates/crpg-sim/src/history.rs` (three spawn wrappers, three zero-event
  setters; no new event variant, no version change)
- `crates/crpg-sim/src/lib.rs` (re-exports)
- `crates/crpg-sim/tests/continuation.rs` (implementer, new),
  `crates/crpg-sim/tests/continuation_contract.rs` (two-keys writer, new)
- `crates/crpg-sim/tests/goldens/` (continuation schedule goldens, new; see
  Tests), `crates/crpg-sim/examples/generate_continuation_golden.rs` (new,
  shares the schedule source)
- `docs/architecture/crpg-sim.md` (a new section "Persisted continuation
  state (T052)"), `crates/crpg-sim/AGENTS.md` (public surface, traps)
- goldens: none moved (absence-skip keeps every continuation-free world
  byte-identical); two new per-target baselines added.

## Readings

## Interface

## Behaviour

## Errors

## Tests

## What must not change

## Dependency edges

## Out of scope

## Open questions

## Agent log

- 2026-11-19 (UTC) · opencode-go/muse-spark-1.3-contributor + T052 draft b · Blind exact-contract draft for persisted dialogue/quest/`Wait` continuation state in `crpg-sim`'s `World`.
