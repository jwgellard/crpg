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
Design decisions this contract makes. Each needs approval with the contract.

1. **Each continuation, open dialogue and quest stage is an entity holding
   one component in its own store.** The spec's 2026-09-30 note already says
   running graphs are entities with a `ScriptContinuation` component
   (`docs/CRPG_ENGINE_SPEC.md:624`), so `World` gains three `ComponentStore`
   fields (`crates/crpg-sim/src/world.rs:104` is the `creatures` precedent,
   `:109` the `movers` one). This reuses the whole store machinery for free:
   despawn-total (`crates/crpg-sim/AGENTS.md:180–187`), no-dangling load
   validation (`crates/crpg-sim/src/world.rs:150`), pair-list serde
   (`crates/crpg-sim/AGENTS.md:205–213`) and absence-skip bytes
   (`crates/crpg-sim/src/world.rs:102–108`). Own entities also keep transfer
   total: the move contract moves one entity's `EntityMeta`, `Transform`,
   record and mover (`crates/crpg-sim/AGENTS.md:148–161`), and a component
   that lived on a game entity would have to extend that contract or fail
   the move (the T139 trap, `crates/crpg-sim/AGENTS.md:594`); a continuation
   entity is never a transfer candidate, so the check is one refusal.
   *Rejected:* one `ScriptState` resource struct with `BTreeMap`s on
   `World` (a second serde/validation idiom beside the stores, and entity
   references inside it would need hand-written dangling checks).
2. **No locals map in this task.** `EventGraph` declares `locals:
   Vec<VarDecl>` (`crates/crpg-data/src/ir.rs:13`), but no value type exists
   anywhere (`crates/crpg-data/src/ir.rs:116`: no saved-continuation type at
   all) and the interpreter that would read or write locals is T053's
   (`docs/IMPLEMENTATION_PLAN.md:283`). T052 stores the resume identity
   (graph, node, remaining ticks) and the dialogue/quest positions; T053
   adds locals values if its design needs them, as a last field.
   *Rejected:* inventing a `LocalValue` enum now (unbounded `String`
   payloads with no cap rule, hashed state no reader reads yet — the T137
   lesson, `crates/crpg-sim/AGENTS.md:514–521`).
3. **The sim owns the countdown; resumption stays upward.** `run_systems`
   gains a last stage, `continuation_system` (stage 5), after movement
   (`crates/crpg-sim/src/tick.rs:64`; appending is routine, reordering is a
   reviewed behaviour change, `crates/crpg-sim/AGENTS.md:355–359`,
   `:609`). Each tick it decrements every positive `remaining` saturating;
   at zero the continuation is due and waits. The sim never interprets, ends
   dialogues, transitions quests, or removes anything: T053 resumes a due
   continuation with a fresh slice budget (D17,
   `tasks/POST-T018-DECISIONS.md:383`–`389`) and T054 advances dialogue and
   quest components. The countdown is pure data mechanics, like the
   encounter clock advancing in its stage
   (`crates/crpg-sim/AGENTS.md:385–391`); execution stays above `crpg-sim`, which still has no
   `crpg-script` edge (`tools/lint/deps.py` stays unedited).
   *Rejected:* no sim stage, host polls and decrements (the host cannot
   mutate authoritative state directly, root `AGENTS.md:13–16`; and the
   spec's 500/500-vs-1000 equivalence gate,
   `docs/CRPG_ENGINE_SPEC.md:853`, runs on ticks, so a tick-driven countdown
   is what the gate covers).
4. **The countdown emits no event.** New `SimEvent` variants need consumers
   (the trap, `crates/crpg-sim/AGENTS.md:350–351`); no consumer of
   "a wait elapsed" exists until T053. Due continuations are found by the
   `continuations_due` query, like `legal_actions` enumeration (options are
   not leases, `docs/architecture/crpg-sim.md:340`).
   *Rejected:* a `WaitElapsed` event (unconsumed vocabulary in the hashed
   queue changes every golden).
5. **The save schema version bumps; nothing migrates.** No `SCHEMA_VERSION`
   constant exists anywhere in the tree
   (`crates/crpg-server/src/save.rs:43`); the save-side versions are
   `SAVE_FORMAT_VERSION` (`crates/crpg-persist/src/envelope.rs:17`),
   `HOST_SAVE_VERSION` (`crates/crpg-server/src/save.rs:43`) and the
   checkpoint pair (`crates/crpg-server/src/checkpoint.rs:54`). The spec
   still demands a schema version in saves
   (`docs/CRPG_ENGINE_SPEC.md:851`). `HOST_SAVE_VERSION` goes 1 → 2; the
   loader accepts 1 and 2 and validates both identically, because absence
   means empty: every new `World` field is skipped when empty
   (`crates/crpg-sim/src/world.rs:83`–`:113` pattern) and defaults on load
   (`:134` pattern), so a version-1 save loads as a world with no
   continuations, no open dialogues and no quest stages. That absence *is*
   the migration. Content renames/removals stay T111's
   (`docs/IMPLEMENTATION_PLAN.md:337`).
   *Rejected:* a third checkpoint version (checkpoints version the host
   wrapper, not the `World` schema); a `WORLD_SCHEMA_VERSION` inside
   `crpg-sim` (no version constant lives beside `World` today, and the
   envelope/host layer is where versions are already enforced).
6. **History journals spawns, not sets; the version never moves for
   continuations.** Spawning a continuation/dialogue/quest-progress entity
   enqueues a legacy `Spawned`, so the wrapper journals one `Spawned` per
   spawn — the `register_creature` precedent
   (`crates/crpg-sim/src/history.rs:643`–`644`), which likewise never
   changes the version. Setters (re-arm a wait, move a dialogue node, change
   a quest stage) rewrite component bytes and journal zero events — the
   movement-operation precedent (`crates/crpg-sim/AGENTS.md:603`);
   `history_hash` still covers the wrapper bytes with no exclusions
   (`crates/crpg-sim/AGENTS.md:135–137`). Records never
   upgrade the version (T139 D12,
   `crates/crpg-sim/src/history.rs:83`–`84`); continuations, dialogues and
   quest stages join that rule: no new `HistoryEvent` variant, so no new
   `is_v2`/`is_v3` arm to forget (`crates/crpg-sim/src/history.rs:175`,
   `:187`).
   *Rejected:* journaling sets as new history events (a version-4 decision
   with no consumer before T053/T054).
7. **Quest progress is one component per quest, keyed by authored quest
   ULID.** A quest's stage is world-global, not per-entity: the component
   holds `quest: Ulid` and `state: Ulid`, and at most one live component per
   quest ULID (load rejects duplicates; spawn of a second fails). MVP needs
   one quest with `started` and `completed`
   (`docs/CRPG_ENGINE_SPEC.md:1520`); the D27f completion rule ("on this
   creature's death, set the quest to completed",
   `tasks/DECISIONS-2026-10-04.md:91`) is T053's trigger to evaluate and
   T054's transition to write — T052 only stores the stage they agree on.
   *Rejected:* quest state on the quest-giver's entity (despawn of the NPC
   would take the quest with it; completion is observed after the enemy's
   death, whose despawn is unrelated).

## Interface
Additions only, in `crates/crpg-sim`. Every public item has a doc comment
(`crates/crpg-sim/AGENTS.md:218`). `Ulid` is `crpg_core`'s
(`crates/crpg-sim/src/creature.rs:12`); `EntityId` likewise. No `HashMap`,
`HashSet`, clock, thread, I/O, float or `std::time` (invariants 5 and 8,
`crates/crpg-sim/AGENTS.md:205–227`); `remaining` counts simulation ticks,
never seconds/rounds/turns (`docs/CRPG_ENGINE_SPEC.md:625`).

### `crates/crpg-sim/src/continuation.rs` (new, re-exported at the crate root)
```rust
/// A script graph paused in `Wait`: which graph, which node, how many ticks
/// are left. Data only; resumption is T053's (Reading 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptContinuation {
    /// The authored `EventGraph.id` (`crates/crpg-data/src/ir.rs:13`).
    pub graph: Ulid,
    /// The authored `Wait` node id (`crates/crpg-data/src/ir.rs:117`).
    pub node: Ulid,
    /// Ticks left before the continuation is due; decremented by stage 5.
    pub remaining: u64,
}

/// An open dialogue: which authored dialogue, which node, who is talking.
/// Node kinds are `crates/crpg-data/src/document.rs:191`'s; checks resolve
/// through `ResolutionRequest` (`docs/CRPG_ENGINE_SPEC.md:647`) in T054.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenDialogue {
    /// The authored `Dialogue.id` (`crates/crpg-data/src/document.rs:155`).
    pub dialogue: Ulid,
    /// The current `DialogueNode.id` (`:176`).
    pub node: Ulid,
    /// The live interlocutor entity (load-checked like any store key).
    pub speaker: EntityId,
}

/// A quest's persisted stage: which authored quest, which state. The state
/// machine itself is T054's (`docs/IMPLEMENTATION_PLAN.md:284`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestProgress {
    /// The authored `Quest.id` (`crates/crpg-data/src/document.rs:239`).
    pub quest: Ulid,
    /// The current `QuestState.id` (`:260`).
    pub state: Ulid,
}
```
`Ulid` serializes symbolically (strings, never interned handles:
`docs/adr/0006-crpg-core-primitives.md:160`,
`docs/adr/0013-combat-simulation-state.md:39`); `EntityId` pair-lists match
the store idiom (`crates/crpg-sim/AGENTS.md:205–213`).

```rust
/// Stage 5: decrement every positive `remaining` saturating. Crate-private;
/// called from `run_systems` after `movement_system`. Never removes, never
/// emits, never interprets (Readings 3–4).
pub(crate) fn continuation_system(world: &mut World);

/// Entities whose continuation is due (`remaining == 0`), ascending
/// `EntityId`. Read-only; conferring nothing (Reading 4).
pub fn continuations_due(world: &World) -> Vec<EntityId>;
```

### `crates/crpg-sim/src/world.rs` — three last fields after `nav`
(`crates/crpg-sim/src/world.rs:110`–`114`; the last-field rule,
`tasks/T139.md:295`):
```rust
// in `pub struct World` (`crates/crpg-sim/src/world.rs:70`), after `nav`:
    #[serde(default, skip_serializing_if = "ComponentStore::is_empty")]
    continuations: ComponentStore<ScriptContinuation>,
    #[serde(default, skip_serializing_if = "ComponentStore::is_empty")]
    dialogues: ComponentStore<OpenDialogue>,
    #[serde(default, skip_serializing_if = "ComponentStore::is_empty")]
    quests: ComponentStore<QuestProgress>,
```
`WorldRepr` (`:126`) gains the three matching entries in the same field
order with `#[serde(default = "ComponentStore::new")]` (`:134` pattern).
Public getters `continuations()`, `dialogues()`, `quests()` beside
`creatures()`/`movers()` (`:1207`, `:1217`); crate-private mutable
accessors beside `:1202`, `:1212`, `:1223`. Public writers for the crates
above sim (T053/T054 call these; no `&mut` store escapes):
```rust
impl World {
    /// Spawn a continuation/dialogue/quest-stage entity; one `Spawned`.
    pub fn spawn_continuation(&mut self, c: ScriptContinuation) -> EntityId;
    pub fn spawn_dialogue(&mut self, d: OpenDialogue) -> EntityId;
    /// Fails when a live `QuestProgress` already names `q.quest` (Reading 7).
    pub fn spawn_quest(&mut self, q: QuestProgress) -> Result<EntityId, ContinuationError>;
    /// Rewriters for T053/T054; change nothing else, enqueue nothing.
    pub fn set_remaining(&mut self, e: EntityId, remaining: u64) -> Result<(), ContinuationError>;
    pub fn set_dialogue_node(&mut self, e: EntityId, node: Ulid) -> Result<(), ContinuationError>;
    pub fn set_quest_state(&mut self, e: EntityId, state: Ulid) -> Result<(), ContinuationError>;
}
```
`despawn_tracked` strips all three stores with the entity
(`tasks/T139.md:584` is the `creatures` precedent); `transfer_entity`
refuses a holder with the new final preflight error (Reading 1; the T139
trap, `crates/crpg-sim/AGENTS.md:594`). Load validation
(`crates/crpg-sim/src/world.rs:150`): every record on a live entity; no
two live `QuestProgress` share a `quest`; every `speaker` is live.

### `crates/crpg-sim/src/history.rs` — thin wrappers, no new variant
```rust
impl HistoryWorld {
    /// Stage a clone, spawn, drain the one legacy `Spawned`, commit one
    /// `Spawned`; version untouched (Reading 6;
    /// `crates/crpg-sim/src/history.rs:623`–`644` is the shape).
    pub fn spawn_continuation(&mut self, c: ScriptContinuation) -> Result<EntityId, HistoryError>;
    pub fn spawn_dialogue(&mut self, d: OpenDialogue) -> Result<EntityId, HistoryError>;
    pub fn spawn_quest(&mut self, q: QuestProgress) -> Result<EntityId, HistoryError>;
    /// Zero-event commits: world bytes change, journal does not (Reading 6).
    pub fn set_remaining(&mut self, e: EntityId, remaining: u64) -> Result<(), HistoryError>;
    pub fn set_dialogue_node(&mut self, e: EntityId, node: Ulid) -> Result<(), HistoryError>;
    pub fn set_quest_state(&mut self, e: EntityId, state: Ulid) -> Result<(), HistoryError>;
}
```
`HistoryWorld::tick` picks up the stage-5 countdown inside the staged
world and commits it with whatever else the tick journals; a tick whose
only change is countdown commits zero new envelopes.

## Behaviour

## Errors

## Tests

## What must not change

## Dependency edges

## Out of scope

## Open questions

## Agent log

- 2026-11-19 (UTC) · opencode-go/muse-spark-1.3-contributor + T052 draft b · Blind exact-contract draft for persisted dialogue/quest/`Wait` continuation state in `crpg-sim`'s `World`.
