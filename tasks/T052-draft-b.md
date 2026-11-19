# T052-draft-b — persisted continuation state (`crpg-sim`)

## Task
T052 adds persisted continuation state to `crpg-sim`'s `World` (D27f,
`tasks/DECISIONS-2026-10-04.md:87`): the data over which the T053 event-IR
interpreter and the T054 dialogue engine and quest state machine will run
(`docs/IMPLEMENTATION_PLAN.md:283`–`284`). A script graph paused in
`Wait(ticks)` (`docs/CRPG_ENGINE_SPEC.md:617`), an open dialogue
(`:642`) and a quest's current stage (`:239`–`260`) all live in the
`World`, survive save/load and server restart, and resume exactly
(`:624`–`625`); the save/load equivalence gate still holds (`:853`).
`crpg-sim` stays free of `crpg-script`: the continuation is world-owned
serializable IR state, and a live Lua stack/coroutine is never saved
(`docs/CRPG_ENGINE_SPEC.md:624`; `crates/crpg-script/src/lib.rs:10`;
`tasks/POST-T018-DECISIONS.md:381`). One part, in `crpg-sim` only.

Citation convention (T153's): a bare `:N` is a line of the file cited
earlier in the same item. Every citation was checked on this branch.

## Parts
| Part | Crate | Scope | Size (source + tests) | S-set | Review |
|---|---|---|---|---|---|
| T052 | `crpg-sim` | continuation/dialogue/quest state: three stores, tick stage 5, save version | ~120 + ~200 suite | yes (`World` touch, `docs/IMPLEMENTATION_PLAN.md:282`) | Claude reviewer mutant run on `src/continuation.rs`, one fix round |

Order: one part. T053 (`crpg-script` interpreter,
`docs/IMPLEMENTATION_PLAN.md:283`) and T054 (dialogue engine and quest state
machine, `:284`) build on it and need no rebase discipline with it.

### T052 owns
- `crates/crpg-sim/src/continuation.rs` (new), `world.rs` (three last
  fields, `WorldRepr`, validation, `despawn_tracked`, transfer preflight,
  getters), `tick.rs` (one stage-5 call), `history.rs` (three spawn
  wrappers, three zero-event setters), `lib.rs` (re-exports)
- `crates/crpg-sim/tests/continuation.rs` (implementer, new),
  `continuation_contract.rs` (two-keys writer, new),
  `tests/support/continuation_schedule.rs` (new),
  `examples/generate_continuation_golden.rs` (new, shares the schedule)
- `docs/architecture/crpg-sim.md` (new section), `crates/crpg-sim/AGENTS.md`
- goldens: none moved; `tests/goldens/continuation_v1.<target>.txt` added.

## Readings
Design decisions this contract makes. Each needs approval with the contract.

1. **Each continuation, open dialogue and quest stage is an entity holding
   one component in its own store.** The spec's 2026-09-30 note already says
   running graphs are entities with a `ScriptContinuation` component
   (`docs/CRPG_ENGINE_SPEC.md:624`), so `World` gains three `ComponentStore`
   fields (`crates/crpg-sim/src/world.rs:104` is the `creatures` precedent,
   `:109` the `movers` one). This reuses the store machinery for free:
   despawn-total, no-dangling load validation, pair-list serde and
   absence-skip bytes (`crates/crpg-sim/AGENTS.md:180–187`,
   `:205–213`; `crates/crpg-sim/src/world.rs:102–108`, `:150`). Own
   entities also keep transfer total: a component on a game entity would
   have to extend the move contract or fail the move (the T139 trap,
   `crates/crpg-sim/AGENTS.md:594`); a continuation entity is never a
   transfer candidate, so the check is one refusal.
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
   (`crates/crpg-sim/AGENTS.md:385–391`); execution stays above `crpg-sim`,
   `crpg-script` edge (`tools/lint/deps.py` stays unedited).
   *Rejected:* no sim stage, host polls and decrements (the host cannot
   mutate authoritative state directly, root `AGENTS.md:13–16`; and the
   spec's 500/500-vs-1000 equivalence gate,
   `docs/CRPG_ENGINE_SPEC.md:853`, runs on ticks, so a tick-driven countdown
   is what the gate covers).
4. **The countdown emits no event.** New `SimEvent` variants need consumers
   (the trap, `crates/crpg-sim/AGENTS.md:350–351`); no consumer of
   a wait elapsing starts before T053. Due continuations are found by the
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
   loader accepts both and validates both identically, because absence
   means empty (the skip-when-empty / default-on-load patterns,
   `crates/crpg-sim/src/world.rs:83`–`:113`, `:134`): a version-1 save
   loads as a world with no continuations, dialogues or quest stages. That
   absence *is* the migration. Content renames stay T111's
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
   ULID.** The component holds `quest: Ulid` and `state: Ulid`; at most one
   live component per quest ULID (load rejects duplicates; a second spawn
   fails). MVP needs one quest with `started` and `completed`
   (`docs/CRPG_ENGINE_SPEC.md:1520`); the D27f completion rule ("on this
   creature's death, set the quest to completed",
   `tasks/DECISIONS-2026-10-04.md:91`) is T053's trigger to evaluate and
   T054's transition to write — T052 only stores the agreed stage.
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
    /// Authored `EventGraph.id` (`crates/crpg-data/src/ir.rs:13`).
    pub graph: Ulid,
    /// The authored `Wait` node id (`crates/crpg-data/src/ir.rs:117`).
    pub node: Ulid,
    /// Ticks left before due; decremented by stage 5.
    pub remaining: u64,
}

/// An open dialogue: which authored dialogue, which node, who is talking
/// (`crates/crpg-data/src/document.rs:191` for the node kinds;
/// `docs/CRPG_ENGINE_SPEC.md:647` for checks; the engine is T054's).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenDialogue {
    /// Authored `Dialogue.id` (`crates/crpg-data/src/document.rs:155`).
    pub dialogue: Ulid,
    /// Current `DialogueNode.id` (`:176`).
    pub node: Ulid,
    /// Live interlocutor entity (load-checked like any store key).
    pub speaker: EntityId,
}

/// A quest's persisted stage (the state machine is T054's,
/// `docs/IMPLEMENTATION_PLAN.md:284`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestProgress {
    /// Authored `Quest.id` (`crates/crpg-data/src/document.rs:239`).
    pub quest: Ulid,
    /// Current `QuestState.id` (`:260`).
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
    /// Spawn wrappers: stage a clone, spawn, drain the one legacy
    /// `Spawned`, commit one `Spawned`; version untouched (Reading 6;
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
`HistoryWorld::tick` commits the stage-5 countdown inside the staged world
with whatever else the tick journals; a countdown-only tick commits zero
new envelopes.

## Behaviour
Normative. Each item names the tests that prove it. "Unchanged" means
`state_hash`, queue bytes, RNG streams, tick and every other store as before
the call. **First failure wins** in every ordered list.

B-1. **Spawns.** `spawn_continuation(c)` / `spawn_dialogue(d)` mint a fresh
entity, insert the record and enqueue exactly one `Spawned` at the world's
tick (`crates/crpg-sim/AGENTS.md:180–187`). `spawn_quest(q)` does the same
unless a live `QuestProgress` already names `q.quest`: then
`Err(DuplicateQuest)` and unchanged. *Tests:* C1, C7.
B-2. **Setters.** The three setters rewrite exactly the named field of the
live record on `e` and enqueue nothing; a dead, absent or wrong-kind `e`
gives `Err(Absent)` and unchanged. *Tests:* C2.
B-3. **Despawn is total.** `despawn` of a holder strips all three stores
with the entity and enqueues exactly one `Despawned`
(`crates/crpg-sim/AGENTS.md:180–187`). *Tests:* C3.
B-4. **Transfer refuses holders.** `transfer_entity` (and the history pair)
of a holder fails with the new final preflight error and publishes neither
side (Reading 1; `crates/crpg-sim/AGENTS.md:594`). The check runs after
every existing check. *Tests:* C4.
B-5. **Stage 5 counts down.** Each `tick` (and `tick_with_movement`, which
delegates), after `movement_system`, decrements every positive `remaining` by exactly
one saturating; zero stays zero; nothing else moves; no event is enqueued
(Readings 3–4). `end_turn` never touches the stores
(`crates/crpg-sim/src/tick.rs:55`). *Tests:* C5, golden G1.
B-6. **Due query.** `continuations_due` returns exactly the holders with
`remaining == 0`, ascending `EntityId`, and changes nothing. *Tests:* C5.
B-7. **Round-trip.** A world with continuations (positive, zero and
large `remaining`), an open dialogue and two quest stages serializes and
deserializes to an equal world: same records, same `state_hash`, same
`continuations_due` (`docs/CRPG_ENGINE_SPEC.md:311`;
`crates/crpg-sim/src/world.rs:102–108). *Tests:* C6.
B-8. **Load rejects the incoherent.** Deserialization fails when any store
names a non-live entity, two live `QuestProgress` share a `quest`, or a
`speaker` is non-live — before any `World` is built
(`crates/crpg-sim/src/world.rs:150`). *Tests:* C6.
B-9. **Save/load equivalence.** A world with a `remaining: 600`
continuation, an open dialogue and a quest stage, run 500 ticks, saved,
loaded and run 500 more, hashes equal to the uninterrupted 1000-tick run —
and the continuation is due at the same tick in both
(`docs/CRPG_ENGINE_SPEC.md:853`;
`crates/crpg-server/tests/host_save.rs:706`–`709`). *Tests:* G1.
B-10. **Waiting outlives a server restart.** Host file-backed save/load
with an armed continuation, an open dialogue and a quest stage: after
reload under a strictly greater incarnation the records, counts and hashes
match the uninterrupted run (Gate 10:
`docs/architecture/crpg-server.md:975`,
`crates/crpg-server/AGENTS.md:293`). *Tests:* G2 (in T052's suite with the
existing host save helpers; no server file changes).
B-11. **History wrappers.** Each spawn wrapper commits exactly one
`Spawned` and leaves `version` untouched; each setter commits zero events
(Reading 6; `crates/crpg-sim/src/history.rs:623`–`644`). A gameplay
failure commits nothing, including the entity allocation. *Tests:* C8.
B-12. **Budgets untouched.** 1024 dispatches and 100,000 units per resumed
slice, depth ≤ 32 (`docs/CRPG_ENGINE_SPEC.md:630`); a `Wait` commits the
slice and a resume gets a fresh budget (D17,
`tasks/POST-T018-DECISIONS.md:383`–`389`). T052 enforces none of it. The
contract fails if it adds a counter or a budget field. *Tests:* C9.

## Errors
| Item | Value |
|---|---|
| second live `QuestProgress` for the same quest | `Err(ContinuationError::DuplicateQuest { quest: Ulid })`, unchanged |
| setter on a dead, absent or wrong-kind entity | `Err(ContinuationError::Absent { entity: EntityId })`, unchanged |
| transfer of a holder of any of the three components | the new final-preflight `AreaError` (Reading 1), neither side published |
| incoherent save (dangling store id, duplicate quest, non-live `speaker`) | load fails before any `World` is built (B-8) |
| history wrapper over a full journal or a gameplay failure | `HistoryError` (`EventLimit`, `ByteLimit`, …: `crates/crpg-sim/src/history.rs:276`–`309`); commits nothing |

One new enum, additive only:
```rust
/// Failures of the continuation/dialogue/quest stores (T052).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContinuationError {
    /// A live `QuestProgress` already names this authored quest.
    DuplicateQuest { quest: Ulid },
    /// No live record of that kind on this entity.
    Absent { entity: EntityId },
}
```
`Display` pins locations in the per-field `spec/*` idiom already used in
this crate (`crates/crpg-sim/AGENTS.md:450–456`). No existing variant changes; `CombatError`,
`AreaError` (one additive variant), `HistoryError` and `HostError` are
otherwise untouched.

## Tests
Public API in `crates/crpg-sim/tests/continuation.rs`; the two-keys writer's
`continuation_contract.rs` is written from this contract alone and may copy
the rig. Every case has a control differing in one input and a "fails if".
Fixed ULIDs (all well-formed 26-char Crockford strings): graph
`000000000000000000000001`, node `000000000000000000000002`, dialogue
`000000000000000000000003`, dnode `000000000000000000000004`, quest
`000000000000000000000005`, qstate `000000000000000000000006`,
qstate2 `000000000000000000000007`. `W(seed)` is `World::new(seed)`;
entities are arena indices 0, 1, 2… (generation 1 while live).

- **C1 `spawns_enqueue_one_spawned`.** `W(1)`: `spawn_continuation({graph,
  node, remaining: 10})` → entity 0; `spawn_dialogue({dialogue, dnode,
  speaker: 0})` → 1; `spawn_quest({quest, qstate})` → 2. Queue holds
  exactly three `Spawned` at tick 0; each store holds one record.
  *Control:* a second `spawn_quest({quest, qstate2})` →
  `Err(DuplicateQuest)`, `len()` still 3, queue still three.
  *Fails if:* a spawn enqueues anything but one `Spawned`, or the duplicate
  publishes.
- **C2 `setters_rewrite_one_field`.** From C1's world:
  `set_remaining(0, 4)`, `set_quest_state(2, qstate2)`;
  `set_dialogue_node(1, dnode)` (same value) leaves bytes unchanged. Queue
  still three `Spawned`. *Control:* `set_remaining(1, 4)` (entity 1 holds
  a dialogue) → `Err(Absent)`, bytes identical.
  *Fails if:* a setter enqueues, touches another record, or accepts a
  wrong-kind entity.
- **C3 `despawn_strips_all_three`.** From C1's world: `despawn(0)` →
  stores empty of entity 0, one `Despawned` appended (queue length 4).
  *Control:* `despawn(7)` (never live) → `false`, queue still 3.
  *Fails if:* the record survives its entity, or a dead id enqueues.
- **C4 `transfer_refuses_holders`.** Two `World::new_in_area(seed, area)`
  worlds (`crates/crpg-sim/AGENTS.md:148–161`): source holds a
  continuation; `transfer_entity` → the new final-preflight error, neither
  side publishes. *Control:* the same move after `despawn` succeeds.
  *Fails if:* the record is silently dropped or an earlier error is
  replaced.
- **C5 `stage_five_counts_down`.** `W(2)` with `remaining: 3` (entity 0)
  and `remaining: 0` (entity 1): three `tick`s → `[0, 0]`;
  `continuations_due` is `[1]` before tick 1, `[0, 1]` after tick 3.
  *Control:* `remaining: u64::MAX` ticks once → `u64::MAX - 1`, never wraps;
  `end_turn` between ticks changes nothing.
  *Fails if:* a tick moves by anything but one, a zero goes nonzero, an
  event is enqueued, or the order is not ascending.
- **C6 `round_trip_and_load_rejection`.** C1's world → JSON → load: equal
  records, hash and due query. Corrupted inputs each fail: a continuation
  on entity 9 (non-live); two quest components sharing `quest`; a dialogue
  with `speaker: 9`. *Control:* the uncorrupted JSON loads.
  *Fails if:* any corruption loads, or the round-trip hash moves.
- **C7 `quest_single_flight_end_to_end`.** MVP shape
  (`docs/CRPG_ENGINE_SPEC.md:1519`–`1526`): `spawn_quest({quest, qstate})`,
  ticks pass, `set_quest_state(e, qstate2)`, `despawn(quest_npc)` — the NPC
  may die without taking the quest (Reading 7). Final: quest reads
  `qstate2`, NPC gone, hash stable across a save/load between the writes.
  *Control:* same run without the `despawn` → identical quest bytes.
  *Fails if:* NPC lifetime affects quest state.
- **C8 `history_wrappers_journal_spawns_only`.** `HistoryWorld` from
  `W(3)`: `spawn_continuation` journals exactly one `Spawned`,
  `version() == 1`; `set_remaining` journals nothing, `version() == 1`;
  both move `history_hash`. A duplicate `spawn_quest` commits nothing.
  *Control:* the same ops on the bare `World` give equal records.
  *Fails if:* a set journals, the version moves, or a failure commits.
- **C9 `no_budgets_in_sim`.** No resumed-slice budget (1024 dispatches,
  100_000 units, depth 32) appears in `crates/crpg-sim/src/`; a due
  continuation ticked 2000 times stays due, `remaining` 0.
  *Fails if:* the sim counts, caps or clears anything.
- **G1 `continuation_golden`.** `tests/support/continuation_schedule.rs`
  (also compiled by `examples/generate_continuation_golden.rs`): seed 42,
  one continuation at `remaining: 600`, one open dialogue, one quest stage,
  plus the T036b movement walk so stage 5 runs beside stage 4. Save at tick
  500, load, run to 1000: per-step `state_hash` lines meet
  `tests/goldens/continuation_v1.<target>.txt`, and equal the uninterrupted
  1000-tick run's hashes step for step (B-9). *Control:* the same schedule
  with no continuation entities reproduces the movement golden's hashes
  exactly (absence-skip: no golden moves). *Fails if:* any line differs.
- **G2 `restart_keeps_waits`.** Host file-backed save at tick 500 with an
  armed `remaining: 100` continuation, an open dialogue and quest stage
  `started`; reload under a greater incarnation; run to 600. Due exactly at
  tick 600 in both runs; dialogue node, quest stage, authority hash and
  capture journal match the uninterrupted run (B-10). *Control:* reload
  with `remaining: 0`: due from the first tick after load. *Fails if:* any
  record, count or hash differs.

Expected survivors (declared, not gaps): the `Absent` arms for entities
that can never be live in a loaded world (rejected at load, B-8); the
saturating decrement at `remaining: 0` (no-op by construction).

## What must not change
- Goldens, byte for byte: every file in `crates/crpg-sim/tests/goldens/`
  (`tasks/T052-facts.md:182` lists them) and every
  `crates/crpg-testkit/goldens/` baseline (`:186`–`:187`). The G1 control
  (a continuation-free run reproduces movement's hashes) is the proof.
- Every existing `crpg-sim` test file and `#[cfg(test)]` case, byte for
  byte and passing; tick order before stage 5
  (`crates/crpg-sim/AGENTS.md:609`); `end_turn` stays pop-only
  (`crates/crpg-sim/src/tick.rs:55`); `state_hash` keeps no exclusions and
  still panics on non-finite floats
  (`crates/crpg-sim/src/hash.rs:10`, `:40`).
- Frozen shapes: `SimEvent` gains no variant (Reading 4);
  `HistoryEvent` gains no variant and `HISTORY_VERSION` stays 1/2/3 with the
  same upgrade rule (`crates/crpg-sim/src/history.rs:70`–`84`);
  `CombatState`, `Combatant`, `Mover`, `NavRevision`, `CreatureState` and
  their load checks are untouched; no existing `World` field changes its
  serde attributes (`crates/crpg-sim/src/world.rs:83`–`113`).
- The `crpg-data` authored types (`Dialogue`, `Quest`, `EventGraph`,
  `NodeBody::Wait`, `crates/crpg-data/src/ir.rs:117`) are read, never
  extended; `crpg-script`'s surface (`ActionBindings`, dispatch,
  `crates/crpg-script/src/lib.rs:15`) is untouched — the
  `UnsupportedWait` refusal stays until T053
  (`crates/crpg-script/src/bindings.rs:181`).
- `HOST_SAVE_VERSION: 1` files still load (Reading 5); envelope v1, the
  checkpoint pair and Gate 10's existing cases are unchanged
  (`crates/crpg-persist/src/envelope.rs:17`,
  `crates/crpg-server/src/checkpoint.rs:54`,
  `crates/crpg-server/tests/host_save.rs:706`–`709`).

## Dependency edges
| Edge | Section | `ALLOWED` |
|---|---|---|
| `crpg-sim` → `crpg-core` (already: `Ulid`, `EntityId`) | `[dependencies]` (normal, path) | permitted: the `crpg-sim` row already reaches all three (`crates/crpg-sim/AGENTS.md:307–312`) |
| `crpg-sim` → `crpg-data` (already: none new) | none — T052 copies no authored type, only persists `Ulid`s | permitted and unused: the edge exists for the combat adapter (`docs/architecture/crpg-sim.md:27–31`); T052 adds no `use` of it |
| `crpg-sim` → `crpg-script` | none — forbidden | would point upward (root `AGENTS.md:28–35`); `tools/lint/deps.py`'s `ALLOWED` table does not permit it and this contract needs no new edge |

No new external crate and no new lockfile package: the three new types use
`Ulid`/`EntityId`, `serde`/`serde_json`/`blake3` as today
(`crates/crpg-sim/AGENTS.md:307–312`). `deps.py` is not edited. One task =
one crate (root `AGENTS.md:57–58`).

## Out of scope
- The T053 interpreter (resuming due continuations, node execution, edge
  order in `docs/CRPG_ENGINE_SPEC.md:629`, fresh slice budgets) and the
  T054 dialogue engine and quest state machine (checks, branching,
  transitions, the D27f death-completes-quest rule in
  `tasks/DECISIONS-2026-10-04.md:91`).
- Locals values (Reading 2); any `crpg.dialogue/1` or `crpg.quest/1`
  fixture (none exist: `tasks/T052-facts.md:134`); the `dialogue_branching`,
  `quest_multistage` and `save_load_stress` fixtures, all named at
  `docs/CRPG_ENGINE_SPEC.md:1502`.
- Format migration beyond absence-means-empty (Reading 5); content
  renames (T111, `docs/IMPLEMENTATION_PLAN.md:337`); autosave wiring
  (`docs/CRPG_ENGINE_SPEC.md:854`); the T076 follow-on
  (`docs/IMPLEMENTATION_PLAN.md:349`).
- `crpg-godot`, dialogue UI and journal (T057,
  `docs/IMPLEMENTATION_PLAN.md:286`); any new lint rule or `ALLOWED` entry.

## Open questions (each with a recommendation)
1. **Should `HOST_SAVE_VERSION` 1 → 2, or should the bump live on the
   checkpoint instead?** *Recommend:* the host save (Reading 5) — saves
   are the spec's versioned unit (`docs/CRPG_ENGINE_SPEC.md:851`) and old
   files load unchanged.
2. **Does G2 belong to T052's suite or to Gate 10's server suite?**
   *Recommend:* keep it in T052's suite using the existing host helpers —
   the behaviour (B-10) is T052's, and no server file is touched.
3. **One preflight error for all three stores, or one per store?**
   *Recommend:* one error — the publisher cares that the entity cannot move,
   not which script state holds it, and one variant keeps the surface small.
4. **Should `continuations_due` return ascending `EntityId` or store order?**
   Store iteration is insertion order while timeline iteration is ascending
   (`crates/crpg-sim/AGENTS.md:198–202`). *Recommend:* ascending — the
   interpreter resumes in a deterministic order independent of spawn
   history, and the sort is over the due set only.

## Agent log

- 2026-11-19 (UTC) · opencode-go/muse-spark-1.3-contributor + T052 draft b · Blind exact-contract draft for persisted dialogue/quest/`Wait` continuation state in `crpg-sim`'s `World`.
