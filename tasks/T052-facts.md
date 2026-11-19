# T052 facts — persisted continuation state (dialogue, quests, waits)

Branch `task/T052-facts-f1`, from `master` at `d1a7c04`. Facts only: every
citation is `path:line` on this branch, for an architect to write the T052
contract from.

## Requirements

- `IR Wait node` (`docs/CRPG_ENGINE_SPEC.md:617`) — `Wait(ticks: u64)` suspends on relative simulation ticks and "persists".
- `Serializable mid-execution` (`docs/CRPG_ENGINE_SPEC.md:624`) — a running graph's `Wait` must survive save/load and server restart; running graphs are entities with a `ScriptContinuation` component; the 2026-09-30 note says continuations are world-owned serializable IR state added by a separate sim task before the interpreter, and a live Lua stack/coroutine is never saved.
- `Tick-based waits` (`docs/CRPG_ENGINE_SPEC.md:625`) — `Wait` stores a relative count of simulation ticks, never seconds/rounds/turns, and resumes exactly after save/load.
- `Deterministic IR order` (`docs/CRPG_ENGINE_SPEC.md:629`) — node execution follows edge order in the file, with no implicit concurrency.
- `IR budgets` (`docs/CRPG_ENGINE_SPEC.md:630`) — 1024 dispatches and 100,000 units per resumed slice, depth ≤ 32; `Wait` commits the slice and a resume gets a fresh slice budget.
- `Server-only graphs` (`docs/CRPG_ENGINE_SPEC.md:631`) — graphs execute on the server; the client receives their effects.
- `Action declarations` (`docs/CRPG_ENGINE_SPEC.md:633`) — `ActionSignatureStore` in `crpg-data` (T029a) holds signatures and validates calls; `crpg-script` (T029b, D17) binds trusted Rust handlers to that identity; campaign data cannot register handlers.
- `Action vocabulary` (`docs/CRPG_ENGINE_SPEC.md:635`) — includes `StartDialogue` and `AdvanceQuest`; each action is a Rust function with a declared JSON-schema signature.
- `Dialogue nodes` (`docs/CRPG_ENGINE_SPEC.md:642`) — `NpcLine`, `PlayerChoice` (with optional `check`), `Jump`, `Link`, `End`.
- `Dialogue checks` (`docs/CRPG_ENGINE_SPEC.md:647`) — a player-choice `check` resolves through the same `ResolutionRequest` pipeline as combat.
- `Dialogue round-trip` (`docs/CRPG_ENGINE_SPEC.md:663`) — JSON is source of truth; the text form is a lossless import/export path.
- `No coroutine waits` (`docs/CRPG_ENGINE_SPEC.md:687`) — no coroutine yields across tick boundaries; long waits use the IR's serializable `Wait`.
- `Whole-World saves` (`docs/CRPG_ENGINE_SPEC.md:311`) — the entire `World` implements `Serialize`/`Deserialize`; saves are world snapshots, round-trip tested every commit.
- `Snapshot format` (`docs/CRPG_ENGINE_SPEC.md:851`) — `SnapshotBackend` serializes the entire `World` plus campaign state with `postcard`+`zstd`; saves carry engine version, campaign id and version, and a schema version.
- `Save/load equivalence` (`docs/CRPG_ENGINE_SPEC.md:853`) — CI loads every fixture campaign, runs 500 ticks, saves, loads, runs 500, and asserts the hash matches an uninterrupted 1000-tick run.
- `Autosave points` (`docs/CRPG_ENGINE_SPEC.md:854`) — autosave on area transition and on encounter end, rolling slots.
- `Continuation tests` (`docs/CRPG_ENGINE_SPEC.md:1486`) — `crpg-script` must prove a graph paused in `Wait` survives save/load.
- `Fixture campaigns` (`docs/CRPG_ENGINE_SPEC.md:1502`) — includes `dialogue_branching`, `quest_multistage` and `save_load_stress`.
- `MVP scope` (`docs/CRPG_ENGINE_SPEC.md:1512`) — one area, two creatures, one NPC, one quest, one weapon, one ruleset.
- `MVP dialogue/quest` (`docs/CRPG_ENGINE_SPEC.md:1519`) — one dialogue with three nodes and one choice that starts a quest.
- `MVP quest states` (`docs/CRPG_ENGINE_SPEC.md:1520`) — one quest with `started` and `completed`, completion triggered by the enemy's death.
- `MVP save gate` (`docs/CRPG_ENGINE_SPEC.md:1526`) — "Save. Quit. Load. State is identical."
- `MVP hook` (`docs/CRPG_ENGINE_SPEC.md:1531`) — the MVP trigger is one line of Lua or a hardcoded quest hook; no visual event graph editor.
- `Phase 8` (`docs/CRPG_ENGINE_SPEC.md:1616`) — continuations that survive save/load and quest reachability analysis are Phase 8 deliverables.
- `Acceptance 13–14` (`docs/CRPG_ENGINE_SPEC.md:1695`) — dialogue engine plus dialogue UI, quest state machine plus journal.
- `Acceptance 19` (`docs/CRPG_ENGINE_SPEC.md:1701`) — the event IR interpreter with serializable `Wait` continuations.
- `T010 IR data` (`docs/CRPG_ENGINE_SPEC.md:2017`) — `crpg-data` owns `Dialogue`, `Quest` and event-IR graph types (`Trigger`/`Node`/action signatures) including tick-count `Wait`, per ADR-0008.
- `Validation duties` (`docs/CRPG_ENGINE_SPEC.md:2025`) — checks include unreachable dialogue nodes and quests with no completion path.
- `T052 row` (`docs/IMPLEMENTATION_PLAN.md:282`) — `crpg-sim`, D27f: "Persisted continuation state for dialogue, quests and waits; `World` touch"; kind `C → I, S`.
- `T053 row` (`docs/IMPLEMENTATION_PLAN.md:283`) — `crpg-script`, prerequisites T029b/T052/T101 draft: event IR interpreter with serializable `Wait` continuations and D17 budgets.
- `T054 row` (`docs/IMPLEMENTATION_PLAN.md:284`) — `crpg-script`, prerequisite T053: dialogue engine and quest state machine "over the existing `crpg-data` dialogue/quest documents".
- `T057 row` (`docs/IMPLEMENTATION_PLAN.md:286`) — `crpg-godot`, prerequisites T051/T054: dialogue UI and quest journal.
- `T111 row` (`docs/IMPLEMENTATION_PLAN.md:337`) — architect, before MVP sign-off: module updates and player saves, what a save keeps when an author renames/moves/removes content; format migration exists, content change does not.
- `T076 row` (`docs/IMPLEMENTATION_PLAN.md:349`) — `crpg-data → crpg-sim`, prerequisite T052: triggers, doors/keys, containers, traps.
- `S-set definition` (`docs/IMPLEMENTATION_PLAN.md:93`) — the serialized set includes "the `World` struct or tick order in `crpg-sim`"; packages marked **S** in §3 are in it (`docs/IMPLEMENTATION_PLAN.md:98`).
- `S golden review` (`docs/IMPLEMENTATION_PLAN.md:415`) — the wave table schedules T052 in wave 3 with "Golden review for T052 if hashes move".
- `D27f` (`tasks/DECISIONS-2026-10-04.md:87`) — the user's "confirm both"; the MVP quest completes through a built-in event-IR rule ("on this creature's death, set the quest to completed"), not Lua (`tasks/DECISIONS-2026-10-04.md:91`).
- `IR types in data` (`docs/adr/0008-event-ownership.md:38`) — event-IR graph types (`Trigger`/`Node`/action signatures) live in `crpg-data` as campaign content; the editor, script compiler, persistence and schema tooling all reach data.
- `No game enum in core` (`docs/adr/0008-event-ownership.md:72`) — rejected: a `SimEvent { Damage, Dialogue, Quest, … }` in `crpg-core` would put combat/dialogue/quest vocabulary in the primitives crate.
- `Vocabulary ownership` (`docs/adr/0011-event-payload-fields.md:36`) — the vocabulary enum lives in the crate that owns it: `SimEvent` in `crpg-sim`, event-IR graph types in `crpg-data`, hooks in `crpg-rules`.
- `Symbolic persistence` (`docs/adr/0006-crpg-core-primitives.md:160`) — interned ids are runtime-only handles; the persisted form is always the string.
- `Persistence is symbolic` (`docs/adr/0013-combat-simulation-state.md:39`) — combat persists symbolic strings, never interned handles.
- `Deterministic continuation` (`docs/adr/0014-combat-encounter-release.md:52`) — a second encounter is "a deterministic continuation, not a byte-repeat of the first".
- `Wait not exercised` (`docs/adr/0005-lua-sandbox-spike.md:118`) — the async/`Wait`-node interaction was not exercised; re-verify once `crpg-script`/the event IR exist (`docs/adr/0005-lua-sandbox-spike.md:123`).
- `Dialogue pause` (`docs/adr/0029-kernel-range-and-time-model.md:318`) — P9: important dialogue requests an `Operator` pause; the handoff says dialogue pauses are left to T054/T057 (`tasks/T141.md:1581`).

## World today

## Saving World today

## Authored dialogue, quest and event data today

## Scripting today

## Determinism rules that bind it

## Agent log
