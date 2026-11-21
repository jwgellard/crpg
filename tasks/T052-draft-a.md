# T052 — persisted continuation state for dialogue, quests and waits (`crpg-sim`)

**Status: blind draft (draft A), 2026-11-19 (UTC).** Written by
opencode-go/deepseek-v4-pro from the fact sheet (`tasks/T052-facts.md`) and the
root `AGENTS.md`. A Claude architect writes the final contract from this and
the other blind draft; nothing here is approved.

Citation convention: every citation is `path:line`, checked on this branch
(`task/T052-draft-a-d1`, base `task/T052-facts-f1` at `d1a7c04`). A bare `:N`
or `:N–M` is a line of the file cited earlier in the same sentence.

Notation: `G`, `W`, `N` are non-nil `Ulid`s; `e0` is an `EntityId` index 0,
generation 1. `Tick::get()` (or `world.tick()`) is the world's current tick.

## Task

Give `crpg-sim`'s `World` persisted state for three things the engine pauses in
the middle of, so each survives a save/load and a server restart and resumes
exactly: (1) a running event-IR graph suspended in a `Wait { ticks }` node
(`docs/CRPG_ENGINE_SPEC.md:617`, `:624–625`), (2) one open dialogue (an
authored `crpg.dialogue/1` being walked, `crates/crpg-data/src/document.rs:155`,
`:191`), and (3) a quest's current stage (an authored `crpg.quest/1`,
`crates/crpg-data/src/document.rs:239`). The continuation is **data in
`World`**: `crpg-sim` stays free of `crpg-script` (the edge points the other
way, `tools/lint/deps.py:52`), so execution stays upward. `crpg-sim` owns the
storage, the serialized shape, the load-time coherence, the hash, and the tick
countdown of a `Wait`; the interpreter that *writes* and *resumes* the
continuation is T053 (`docs/IMPLEMENTATION_PLAN.md:283`), which reaches this
state only through the sim controllers named in "Interface". No live Lua
stack or coroutine is ever saved (`docs/CRPG_ENGINE_SPEC.md:624`,
`tasks/POST-T018-DECISIONS.md:381`).

T052 is one `crpg-sim` part, marked `C → I, S` and "`World` touch"
(`docs/IMPLEMENTATION_PLAN.md:282`): it touches the `World` struct and tick
order, which are in the serialized set (`docs/IMPLEMENTATION_PLAN.md:93`,
`:98`), so the `World` struct change and the new tick stage are reviewed, and
the wave table schedules "Golden review for T052 if hashes move"
(`docs/IMPLEMENTATION_PLAN.md:415`). The new state is absent in every existing
world, so no existing golden moves (see "What must not change").

## Parts

| Part | Crate | Scope | Size (source + tests) | S-set | Review |
|---|---|---|---|---|---|
| T052 | `crpg-sim` | the `World` fields, their serde/load shape, the stage-2 wait countdown, the sim controllers and error, docs | ~260 + ~380 | **yes** (`World` struct + tick order, `docs/IMPLEMENTATION_PLAN.md:93`) | Claude reviewer; golden review if hashes move (`:415`) |

Order: one part. It must not run beside another `World`/tick-order edit
(`docs/IMPLEMENTATION_PLAN.md:86–98`).

### T052 owns

- `crates/crpg-sim/src/continuation.rs` (new module: the continuation,
  dialogue and quest state types, `ContinuationError`, the countdown system)
- `crates/crpg-sim/src/lib.rs` (the re-exports of "Interface")
- `crates/crpg-sim/src/world.rs` (the three `World` fields after `nav`, the
  `WorldRepr` entries, the load checks, the public getters)
- `crates/crpg-sim/src/tick.rs` (one call in `run_systems`, stage 2)
- `crates/crpg-sim/tests/continuation.rs` (implementer suite, new),
  `crates/crpg-sim/tests/continuation_contract.rs` (two-keys writer, new)
- `docs/architecture/crpg-sim.md` (a new section "Persisted continuation
  state (T052)"), `crates/crpg-sim/AGENTS.md` (public surface, invariants,
  traps)
- goldens: none new, none moved.

## Readings

Design decisions this contract makes. Each needs approval with the contract.

1. **The continuation is world-owned data; the sim owns storage and the
   countdown, the interpreter owns execution.** D17 records the rule: "Later
   graph/interpreter tasks use world-owned serializable IR continuations in a
   separate earlier sim task. Never store live Lua stack/coroutine in a save"
   (`tasks/POST-T018-DECISIONS.md:381`), and a `Wait` "commits a valid slice
   and serializes continuation; resume receives a new slice budget"
   (`:389`). The "separate earlier sim task" is this one: T052 stores and
   counts, T053 interprets. *Rejected:* a sim stage that calls the interpreter
   (a `crpg-sim → crpg-script` edge, forbidden, `tools/lint/deps.py:52`); a
   saved Lua/coroutine (`docs/CRPG_ENGINE_SPEC.md:624`, `:687`).
2. **Three separate `World` fields, not one continuation enum.** A running
   graph, an open dialogue and a quest stage are three vocabularies with
   different cardinalities and keys, and ADR-0008 keeps mechanism down and
   vocabulary up (`docs/adr/0008-event-ownership.md:38`, `:72`). So they are
   three fields, each absence-skip so a world without them is byte-identical
   to today (`crates/crpg-sim/src/world.rs:102–108`). *Rejected:* one merged
   `SimEvent`-like enum (folds three consumers into one hashed blob); a hash
   exclusion (the exclusion list is empty by design, `crates/crpg-sim/src/hash.rs:10`).
3. **A `Wait` countdown is a stage-2 sim system that decrements `remaining`
   each tick; due is `remaining == 0`.** The spec says `Wait` stores a
   relative count of simulation ticks and "resumes exactly after save/load"
   (`docs/CRPG_ENGINE_SPEC.md:625`), and stage 2 is the "timers, durations,
   refreshes" stage (`crates/crpg-sim/src/tick.rs:65`). The save/load
   equivalence gate runs 500 ticks, saves, loads, runs 500 more
   (`docs/CRPG_ENGINE_SPEC.md:853`); the countdown must therefore advance
   *inside* `tick` so it is hashed state, deterministic per tick. *Rejected:*
   the interpreter decrementing between ticks (outside the 500-tick gate, and
   not tied to simulation ticks); storing an absolute resume tick (the IR
   `Wait` is "relative … no absolute scheduling", `crates/crpg-data/src/ir.rs:117`).
4. **No save schema bump: the new fields are absence-compatible.** Each new
   field is `#[serde(default, skip_serializing_if = "…is_empty"/"Option::is_none")]`
   and joins `World`/`WorldRepr` as the last fields, exactly as `creatures`
   (T139, `tasks/T139.md:568`) and `movers`/`nav` (T036,
   `crates/crpg-sim/src/world.rs:110–114`) did. An old save (no such field)
   loads with the default empty value; a world that never opens a dialogue or
   starts a quest serializes without those keys. So `HOST_SAVE_VERSION`
   (`crates/crpg-server/src/save.rs:43`), `CHECKPOINT_VERSION`
   (`crates/crpg-server/src/checkpoint.rs:54`) and `SAVE_FORMAT_VERSION`
   (`crates/crpg-persist/src/envelope.rs:17`) are unchanged, and there is no
   migration (there is no `SCHEMA_VERSION` constant to migrate, see Open
   question 1). *Rejected:* a version bump (rejects old saves for no
   behavioural gain, and the spec §851 "schema version" has no constant in the
   tree to bump, `tasks/T052-facts.md:100–101`).
5. **Load validation is structural, not content.** `World` deserialization
   already rejects dangling stores and cross-field incoherence
   (`crates/crpg-sim/src/world.rs:150–234`). The new checks are structural
   only: continuation keys live, `remaining` any `u64`, `graph`/`node`/
   `dialogue`/`quest`/`stage` non-nil, quest keys unique (the map enforces
   it). Whether a `graph`/`dialogue`/`node` ULID names real campaign content
   is the host's and interpreter's concern (T027c-style), because the sim
   holds one area's runtime state, not the campaign. *Rejected:* the sim
   reading authored `Dialogue`/`Quest`/`EventGraph` at load (it does not hold
   them, and would couple the sim's save to `crpg-data` document lookup).
6. **Mutators are sim controllers, never raw store escapes.** `crpg-script`
   has no `&mut World` today (`InvocationContext` is read-only,
   `crates/crpg-script/src/bindings.rs:53`); its only mutations go through
   sim controllers like `perform_action`. So T052 adds public controllers
   (`set_continuation`, `clear_continuation`, `open_dialogue`,
   `advance_dialogue`, `close_dialogue`, `set_quest_stage`) that validate and
   mutate atomically, and read-only getters. *Rejected:* exposing
   `script_continuations_mut`/`dialogue_mut`/`quests_mut` publicly (breaks
   the crate's no-raw-mutation discipline and lets callers build incoherent
   saves the loader then rejects).
7. **Quest progress is a `BTreeMap` keyed by quest ULID, ascending.** Quests
   are authored objects, not entities, so they are keyed by `Ulid`, not
   `EntityId`; `BTreeMap` makes iteration independent of insertion order,
   matching the crate's ascending-ULID conventions (abilities, effects,
   `crates/crpg-sim/AGENTS.md:139`). *Rejected:* a `ComponentStore` keyed by
   synthetic entities (quests are not entities); an `IndexMap` (insertion-order
   coupling, against the `BTreeMap`/sorted-map preference of the crate).
8. **The countdown visits continuations in ascending `EntityId` order.** The
   movement stage already visits movers in ascending `EntityId`, not store
   order, because a reused index makes the two differ
   (`crates/crpg-sim/AGENTS.md:612`). The countdown's per-entry decrement is
   independent, so order does not change its output today, but pinning it
   keeps determinism by construction if a later stage observes the due set.
   *Rejected:* store (insertion) order, which is documented "not sorted"
   (`crates/crpg-sim/AGENTS.md:336`).

## Interface

Additions only. Every public item has a doc comment (`#![warn(missing_docs)]`,
`crates/crpg-sim/AGENTS.md:218`). No `HashMap`/`HashSet`, no `f32`/`f64`, no
clock, thread or I/O (invariants 5 and 8, `crates/crpg-sim/AGENTS.md:205`,
`:220`). No `crpg-script` type or import appears anywhere in `crpg-sim`.

### `crates/crpg-sim/src/continuation.rs`

```rust
use crpg_core::{EntityId, Fx16_16, Ulid};
use std::collections::BTreeMap;

/// The runtime subject a trigger names (T052): a sim-owned mirror of the
/// authored [`Trigger`] (`crates/crpg-data/src/ir.rs:44`) with the entity the
/// trigger concerns where it concerns one. Persisted and hashed in full.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TriggerContext {
    /// Entering an area.
    AreaEnter { entity: EntityId },
    /// Leaving an area.
    AreaExit { entity: EntityId },
    /// Interacting with an object.
    Interact { entity: EntityId },
    /// Entering a dialogue node.
    DialogueNode { entity: EntityId },
    /// Changing quest state.
    QuestStateChange { quest: Ulid, state: Ulid },
    /// Relative tick timer.
    Timer { ticks: u64 },
    /// Starting combat.
    CombatStart,
    /// Ending combat.
    CombatEnd,
    /// Death event.
    Death { entity: EntityId },
    /// Acquiring an item.
    ItemAcquired { entity: EntityId },
    /// Extension vocabulary trigger.
    Custom { id: String },
}

/// A graph-local value, the sim-owned mirror of `crpg-data`'s `DataValue`
/// (`crates/crpg-data/src/types.rs:163`): persisted state never holds a
/// `crpg-data` wire type (T137, `crates/crpg-sim/AGENTS.md:515`). Deterministic
/// by construction: `BTreeMap` sorted, `Fx16_16` integer, no float.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContinuationValue {
    Bool(bool),
    Integer(i64),
    Unsigned(u64),
    Fixed(Fx16_16),
    Text(String),
    ObjectRef(Ulid),
    List(Vec<ContinuationValue>),
    Map(BTreeMap<String, ContinuationValue>),
}

/// A running event-IR graph suspended in a `Wait` (`crates/crpg-data/src/ir.rs:117`):
/// the resumable position the interpreter (T053) needs. The interpreter writes
/// and clears it; the sim counts `remaining` down (Reading 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptContinuation {
    /// Authored graph identity (`EventGraph.id`).
    pub graph: Ulid,
    /// The node whose outgoing edge is the resume point (the `Wait` node).
    pub node: Ulid,
    /// Remaining ticks of the current `Wait`; zero means due to resume.
    pub remaining: u64,
    /// Graph-local variable bindings, in the graph's `locals` declaration order.
    pub locals: Vec<(String, ContinuationValue)>,
    /// The initiating trigger and its subject.
    pub trigger: TriggerContext,
}

/// The one open dialogue: the authored dialogue being walked and its current node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DialogueState {
    /// Authored dialogue identity (`Dialogue.id`, `crates/crpg-data/src/document.rs:155`).
    pub dialogue: Ulid,
    /// Current node identity (a `DialogueNode.id`).
    pub node: Ulid,
}

/// Progress on one quest: the authored state it is currently in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestProgress {
    /// Current state identity (a `QuestState.id`, `crates/crpg-data/src/document.rs:260`).
    pub stage: Ulid,
}

/// A continuation/dialogue/quest mutation that was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContinuationError {
    /// `set_continuation` named an entity that is not live.
    NotLive { id: EntityId },
    /// A required authored identity was the nil ULID (`crates/crpg-core/src/ulid.rs:21`).
    NilId { field: &'static str },
    /// `open_dialogue` while a dialogue is already open.
    DialogueOpen,
    /// `advance_dialogue`/`close_dialogue` while none is open.
    NoDialogue,
}
```

`Display` for `ContinuationError` (lowercase, the crate's convention):
`"script continuation for non-live {id}"`, `"nil {field}"` (field one of
`graph`, `node`, `dialogue`, `quest`, `stage`), `"a dialogue is already open"`,
`"no dialogue is open"`.

### `crates/crpg-sim/src/world.rs` — the three `World` fields (after `nav`,
`crates/crpg-sim/src/world.rs:114`)

```rust
    /// Running event-IR graphs paused in `Wait` (T052). Absent (empty) in
    /// worlds with no running graph, so those serialize byte-identically; a
    /// compatibility shape, not a hash exclusion.
    #[serde(default, skip_serializing_if = "ComponentStore::is_empty")]
    script_continuations: ComponentStore<ScriptContinuation>,
    /// The one open dialogue, if any (T052).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dialogue: Option<DialogueState>,
    /// Active quest progress, keyed by authored quest id, ascending (T052).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    quests: BTreeMap<Ulid, QuestProgress>,
```

`WorldRepr` gains the matching entries in the same order
(`crates/crpg-sim/src/world.rs:126–148`):

```rust
    #[serde(default = "ComponentStore::new")]
    script_continuations: ComponentStore<ScriptContinuation>,
    #[serde(default)]
    dialogue: Option<DialogueState>,
    #[serde(default)]
    quests: BTreeMap<Ulid, QuestProgress>,
```

### `crates/crpg-sim/src/world.rs` — the controllers and getters

```rust
impl World {
    /// The running-graph continuations, read-only.
    pub fn script_continuations(&self) -> &ComponentStore<ScriptContinuation>;
    /// The open dialogue, if any.
    pub fn dialogue(&self) -> Option<&DialogueState>;
    /// Active quest progress.
    pub fn quests(&self) -> &BTreeMap<Ulid, QuestProgress>;

    /// Write or replace the running graph's continuation for `id`. Requires a
    /// live entity (`NotLive`) and non-nil `graph`/`node` (`NilId`). The
    /// interpreter spawns the entity first via `spawn`.
    pub fn set_continuation(&mut self, id: EntityId, c: ScriptContinuation)
        -> Result<(), ContinuationError>;
    /// Remove a finished graph's continuation. Returns the removed value, or
    /// `None` if `id` held none (a dead id is `None`, as `ComponentStore::remove`).
    pub fn clear_continuation(&mut self, id: EntityId) -> Option<ScriptContinuation>;

    /// Open a dialogue at `node` (`DialogueOpen` if one is already open;
    /// `NilId` on a nil `dialogue` or `node`).
    pub fn open_dialogue(&mut self, dialogue: Ulid, node: Ulid)
        -> Result<(), ContinuationError>;
    /// Move the open dialogue to `node` (`NoDialogue`; `NilId` on a nil `node`).
    pub fn advance_dialogue(&mut self, node: Ulid) -> Result<(), ContinuationError>;
    /// Close the open dialogue, returning the state it had (`NoDialogue`).
    pub fn close_dialogue(&mut self) -> Option<DialogueState>;

    /// Set (or replace) a quest's current stage (`NilId` on a nil `quest` or
    /// `stage`). Idempotent: setting the current stage again changes nothing.
    pub fn set_quest_stage(&mut self, quest: Ulid, stage: Ulid)
        -> Result<(), ContinuationError>;
}
```

`despawn`/`despawn_tracked` gain `self.script_continuations.remove(id)` beside
the existing `self.creatures.remove(id)` (`crates/crpg-sim/src/world.rs`,
the T139 mechanic `tasks/T139.md:584`); a despawned graph is not resumed.
`transfer_entity`/`transfer_history_entity` refuse an entity holding a
continuation (new `AreaError::ScriptContinuation`, "Errors").

### `crates/crpg-sim/src/tick.rs` — the countdown (stage 2, after the clock)

```rust
    // §10 stage 2 (timers, durations, refreshes) — T138: the encounter clock.
    encounter_clock_system(world, facts);
    // §10 stage 2 (timers, durations, refreshes) — T052: script-wait countdown.
    crate::continuation::script_wait_system(world, facts);
```

`script_wait_system` is crate-private (`pub(crate) fn script_wait_system(world:
&mut World, _facts: &mut TickFacts)`): visit `world.script_continuations()` in
ascending `EntityId`, and for each entry with `remaining > 0` set it to
`remaining - 1`; entries at `0` are left alone. No event, no RNG, no history
event, no history-version change.

## Behaviour

Normative. Each item names the tests that prove it. "Unchanged" means the
world's bytes and `state_hash` are what they were before the call. **First
failure wins** in every ordered check. `world.tick()` is the current tick.

B-1. **Fields and absence-skip.** `World` gains `script_continuations`,
   `dialogue` and `quests` as the last three fields, after `nav`
   (`crates/crpg-sim/src/world.rs:114`), each absent by default. A world that
   never opens a dialogue, starts a quest or runs a graph serializes
   byte-identically to today; every existing golden is unchanged. *Tests:* C1.

B-2. **Round-trip.** A world holding a continuation, an open dialogue and
   quest progress serializes and deserializes to a value `==` the original,
   and `state_hash` of the loaded world equals `state_hash` of the saved one
   (the hash has no exclusions, `crates/crpg-sim/src/hash.rs:10`, `:37`).
   *Tests:* C2.

B-3. **Countdown** (Reading 3). On every `tick`, `script_wait_system` (stage
   2, after `encounter_clock_system`) visits each continuation in ascending
   `EntityId` and, when `remaining > 0`, sets it to `remaining - 1`; at `0` it
   is left alone. A continuation written with `remaining = N` in an
   interpreter pass after tick `T` reaches `remaining == 0` on tick `T + N`.
   *Tests:* C3, C4.

B-4. **Due is `remaining == 0`.** A continuation written with `remaining == 0`
   (a `Wait(0)`) is due immediately and is never decremented below zero
   (`u64`, saturating). The sim never removes a due continuation: resume and
   removal are the interpreter's (`clear_continuation`). *Tests:* C4.

B-5. **Controllers validate before mutation.** `set_continuation` on a dead id
   returns `NotLive { id }`, and on a nil `graph` or `node` returns `NilId`,
   each with the world unchanged. `open_dialogue` while one is open returns
   `DialogueOpen`; `advance_dialogue`/`close_dialogue` with none open returns
   `NoDialogue`; `set_quest_stage` with a nil `quest` or `stage` returns
   `NilId` — every failure changes nothing. *Tests:* C5, C6.

B-6. **Writes replace; clears return.** `set_continuation` replaces an
   existing continuation for the same id; `set_quest_stage` replaces a quest's
   stage (setting the same stage again is a no-op); `clear_continuation`
   returns the removed continuation or `None`; `close_dialogue` returns the
   closed state and clears. *Tests:* C7.

B-7. **Despawn is total.** Despawning an entity holding a continuation removes
   the continuation with the entity (the T139 mechanic,
   `tasks/T139.md:584`), so no dangling continuation survives, exactly as no
   dangling creature record does (`crates/crpg-sim/AGENTS.md:180–189`). *Tests:* C8.

B-8. **Transfer refuses.** `transfer_entity`/`transfer_history_entity` of an
   entity holding a continuation fails `AreaError::ScriptContinuation` and
   publishes neither side (a graph's trigger names the source area; it is not
   silently dropped, `crates/crpg-sim/AGENTS.md:288–293`). *Tests:* C9.

B-9. **Load validation is structural.** `World::deserialize` rejects a
   continuation for a non-live entity (`dangling script continuation for
   non-live {id}`), a nil `graph`/`node`/`dialogue`/`quest`/`stage`, and
   (by construction) duplicate quest keys. A save with well-formed, live state
   loads and hashes identically. Content existence is not checked here
   (Reading 5). *Tests:* C10.

B-10. **Determinism and the equivalence gate.** The stores and the countdown
   are pure and hashed in full: 500 ticks, save, load, 500 more yields the
   same per-tick `state_hash` sequence as 1,000 uninterrupted ticks, because
   the countdown advances only inside `tick`. The whole-campaign gate is Gate
   10 (`crates/crpg-server/tests/host_save.rs:706`, `:524`); T052 proves the
   sim half. *Tests:* C11.

B-11. **History neutrality.** `HistoryWorld::tick` over a world holding a
   continuation decrements its `remaining` but journals no new event and never
   raises the wrapper version (a continuation is not a v2/v3 event;
   `commit` is the only version writer, `crates/crpg-sim/AGENTS.md:374`).
   *Tests:* C12.

## Errors

| Item | Value |
|---|---|
| `set_continuation` on a dead id | `Err(ContinuationError::NotLive { id })` (`Display` `script continuation for non-live {id}`) |
| a nil authored identity (`graph`, `node`, `dialogue`, `quest`, `stage`) | `Err(ContinuationError::NilId { field })` (`nil {field}`) |
| `open_dialogue` while one is open | `Err(ContinuationError::DialogueOpen)` (`a dialogue is already open`) |
| `advance_dialogue`/`close_dialogue` with none open | `Err(ContinuationError::NoDialogue)` (`no dialogue is open`) |
| `transfer_entity`/`transfer_history_entity` of an entity holding a continuation | `Err(AreaError::ScriptContinuation)` (`Display` `cannot transfer an entity holding a script continuation`), checked after the existing `InvalidTransform` and before the history pair's `SourceHistory` |
| load: a continuation for a non-live entity | `serde::de::Error::custom("dangling script continuation for non-live {id}")` |
| load: a nil `graph`/`node`/`dialogue`/`quest`/`stage` | `serde::de::Error::custom("nil {field}")` |

`AreaError` gains exactly one variant, `ScriptContinuation` (fieldless), the
last variant. No other existing error type, `Display` text, variant or
precedence changes.

## Tests

`crates/crpg-sim/tests/continuation.rs` (implementer) plus
`continuation_contract.rs` (two-keys writer, written from this contract
alone). Fixtures are built in-code from `World::new(0)`; ULIDs are
`Ulid::from_parts` so expectations are by equality, never by a hand-encoded
base32 string. `G = from_parts(1,1)`, `W = from_parts(1,2)`, `D =
from_parts(2,1)`, `DN = from_parts(2,2)`, `Q = from_parts(3,1)`, `Q2 =
from_parts(3,2)`, `S1 = from_parts(3,3)`, `S2 = from_parts(3,4)` (so `Q < Q2`).
`c(N, trigger)` is `ScriptContinuation { graph: G, node: W, remaining: N,
locals: vec![("x".into(), ContinuationValue::Unsigned(1))], trigger }`.

- **C1 `empty_world_is_byte_identical`.** `state_hash(World::new(0))` and the
  canonical JSON equal the values today (no `script_continuations`, `dialogue`
  or `quests` key appears). *Control:* a world with one continuation serializes
  to JSON that contains `"script_continuations"`. *Fails if:* an empty world
  gains any of the three keys (a golden moves).
- **C2 `round_trip_preserves_hash`.** `World::new(0)`, spawn `e0`,
  `set_continuation(e0, c(5, Death { entity: e0 }))`, `open_dialogue(D, DN)`,
  `set_quest_stage(Q, S1)`; `h = state_hash(world)`; serialize → deserialize →
  `loaded == world` and `state_hash(&loaded) == h`. *Control:* `c(6, …)`
  instead of `c(5, …)` gives a different `h`. *Fails if:* the round-trip is not
  `==` or the hash changes across it.
- **C3 `countdown_reaches_zero`.** `set_continuation(e0, c(3, …))`; tick 1:
  `remaining == 2`; tick 2: `1`; tick 3: `0`; tick 4: `0` (saturating, never
  below zero). *Control:* `c(0, …)` stays `0` through tick 1. *Fails if:* a
  decrement is skipped, wraps below zero, or a due (`0`) entry is decremented.
- **C4 `due_is_not_removed`.** `set_continuation(e0, c(0, …))`, tick once:
  `remaining == 0` and `script_continuations().contains(e0)` is still `true`
  (the sim never resumes/removes; `clear_continuation` does). *Fails if:* the
  sim removes a due continuation.
- **C5 `controllers_validate_before_mutation`.** Each rejected call leaves
  `state_hash` unchanged: `set_continuation(dead_id, c(1, …))` →
  `Err(NotLive { id: dead_id })`; `set_continuation(e0, ScriptContinuation {
  graph: Ulid::NIL, ..c(1, …) })` → `Err(NilId { field: "graph" })`;
  `open_dialogue(D, DN)` then `open_dialogue(D, DN)` again →
  `Err(DialogueOpen)`; `advance_dialogue(DN)` on a fresh world →
  `Err(NoDialogue)`; `close_dialogue()` on a fresh world → `None`;
  `set_quest_stage(Q, Ulid::NIL)` → `Err(NilId { field: "stage" })`. *Control:*
  each valid counterpart succeeds. *Fails if:* a rejected call mutates anything.
- **C6 `writes_replace_and_clear`.** `set_quest_stage(Q, S1)` →
  `quests() == {Q: {stage: S1}}`; `set_quest_stage(Q, S2)` → `{Q: {stage: S2}}`;
  `set_quest_stage(Q, S2)` again is a no-op (hash unchanged);
  `set_quest_stage(Q2, S1)` → two entries, `Q` before `Q2` (ascending).
  `set_continuation(e0, c(1, …))` then `set_continuation(e0, c(2, …))` →
  `c(2, …)`; `clear_continuation(e0) == Some(c(2, …))`;
  `clear_continuation(e0) == None`. `open_dialogue(D, DN)` then
  `close_dialogue() == Some(DialogueState { dialogue: D, node: DN })` and
  `dialogue().is_none()`. *Fails if:* replace does not replace, a clear returns
  the wrong value, or quest iteration is not ascending by key.
- **C7 `despawn_removes_continuation`.** spawn `e1`, `set_continuation(e1, c(1, …))`,
  `despawn(e1) == true`, then `script_continuations().contains(e1) == false` and
  `assert_no_dangling` holds. *Control:* despawning an entity with no
  continuation leaves the store unchanged. *Fails if:* a continuation survives
  its entity's despawn.
- **C8 `transfer_refuses_continuation`.** Two bound worlds
  (`World::new_in_area(0, A)` / `(0, B)`); source has `e0` with a transform and
  `c(1, …)`; `transfer_entity(&mut src, &mut dst, e0)` →
  `Err(AreaError::ScriptContinuation)` and both worlds are unchanged.
  *Control:* the same source without the continuation transfers (the transform
  is copied). *Fails if:* the continuation is dropped or the transfer
  half-applies.
- **C9 `load_rejects_incoherent`.** JSON whose `script_continuations` pair-list
  names a non-live id fails with `dangling script continuation for non-live`;
  JSON with `graph: Ulid::NIL` fails with `nil graph`. *Control:* the same JSON
  with a live id and non-nil ids loads. *Fails if:* an incoherent save loads.
- **C10 `split_run_equivalence`.** `World::new(0)`, spawn `e0`,
  `set_continuation(e0, c(100, Death { entity: e0 }))`. Record
  `state_hash` for 300 ticks in one run; in another, 150 ticks, serialize,
  deserialize, 150 more. The two 300-length hash sequences are equal
  element-for-element. *Control:* a build where the countdown is dropped (the
  `script_wait_system` call removed) diverges at the tick `remaining` would
  have reached 0 (tick 100). *Fails if:* the split sequence differs from the
  uninterrupted one.
- **C11 `history_does_not_upgrade`.** A version-1 `HistoryWorld` loaded from
  JSON whose world holds a continuation: one `HistoryWorld::tick()` decrements
  `remaining` but `version() == 1` and the journal gains no envelope (a tick
  with no encounter emits nothing). *Control:* the same world with no
  continuation also stays version 1. *Fails if:* a continuation makes `commit`
  raise the version or journal anything.

The two-keys writer's `continuation_contract.rs` is written from this
contract alone and covers C1–C11 with its own controls.

## What must not change

- Goldens, byte for byte (none added, none moved): every
  `crates/crpg-sim/tests/goldens/{history_v1,history_v2,history_v3,movement_v1,surface_v1}.<target>.txt`
  and every `crates/crpg-testkit/goldens/*_rust-1.98.0_<target>_test-default.golden`
  (`tasks/T052-facts.md:182`, `:186`). No existing world, schedule or rule
  opens a dialogue, starts a quest or runs a graph — no campaign fixture uses
  `crpg.dialogue/1` or `crpg.quest/1` (`tasks/T052-facts.md:134`) — so the new
  state is absent everywhere and no hash moves.
- The existing tick order, byte for byte except the one appended stage-2 call:
  `encounter_clock_system` (stage 2), `timeline_system` (stage 3),
  `movement_system` (stage 4) keep their positions
  (`crates/crpg-sim/src/tick.rs:64–71`); adding a stage is a reviewed change
  (`crates/crpg-sim/AGENTS.md:609`).
- Every existing `crpg-sim` test file and its passing status; the `World`
  field order for the fields before `script_continuations` (the three new
  fields are the last, `tasks/T139.md:295`); `WorldRepr` order for existing
  fields; `World::deserialize`'s existing checks and precedence.
- The save versions: `SAVE_FORMAT_VERSION` (`crates/crpg-persist/src/envelope.rs:17`),
  `HOST_SAVE_VERSION` (`crates/crpg-server/src/save.rs:43`),
  `CHECKPOINT_VERSION` / `CHECKPOINT_VERSION_HOST_RECORDS`
  (`crates/crpg-server/src/checkpoint.rs:54`, `:56`). No migration and no
  `SCHEMA_VERSION` is introduced.
- `HistoryEvent`'s variants and `is_v2`/`is_v3`, `commit`'s version condition,
  `HISTORY_VERSION` (still 3), and the `history_v1/v2/v3` goldens. The
  countdown journals nothing and never upgrades a wrapper.
- `crates/crpg-script`, `crates/crpg-data`, `crates/crpg-core`,
  `crates/crpg-rules`, `tools/lint/deps.py`, `tools/lint/determinism.py`,
  `rust-toolchain.toml`, `crpg-contracts`, `Cargo.lock`, `deny.toml`.

## Dependency edges

No new edge. T052 adds no dependency: the new types use `crpg-core`
(`EntityId`, `Ulid`, `Fx16_16`) and the sim-owned `ContinuationValue`/
`TriggerContext` mirrors; every crate reached is already an allowed
`crpg-sim` dependency (`crpg-core`, `crpg-data`, `crpg-rules`,
`tools/lint/deps.py:50`). No new external crate, so no H-dep approval and no
`Cargo.lock` change. `crpg-script`'s edge to `crpg-sim`
(`tools/lint/deps.py:52`) is unchanged and points the other way, which is what
keeps `crpg-sim` free of `crpg-script`.

## Out of scope

- The event-IR interpreter, its slice budgets (1024 dispatches, 100,000 units,
  depth ≤ 32, `docs/CRPG_ENGINE_SPEC.md:630`), and the resume of a due
  continuation — T053 (`docs/IMPLEMENTATION_PLAN.md:283`).
- The dialogue engine and quest state machine over the authored documents —
  T054 (`docs/IMPLEMENTATION_PLAN.md:284`).
- The `StartDialogue`/`AdvanceQuest` action *bindings* (T029b already stores
  them; nothing here binds or dispatches them, `crates/crpg-script/src/lib.rs:3`).
- Trigger *firing* (who turns a `Death`/`Timer` into a running graph), quest
  reachability analysis (Phase 8, `docs/CRPG_ENGINE_SPEC.md:1616`), and the
  built-in "on this creature's death, set quest completed" rule (D27f,
  `tasks/DECISIONS-2026-10-04.md:91`): these are interpreter/engine behaviour.
- Autosave scheduling (`docs/CRPG_ENGINE_SPEC.md:854`), the `crpg-persist`
  envelope, host checkpoint/save code, and any `SCHEMA_VERSION` constant
  introduction.
- The dialogue UI and quest journal (T057, `docs/IMPLEMENTATION_PLAN.md:286`).

## Open questions

Each is for the architect, with this draft's recommendation.

1. **The spec §851 "schema version" has no constant in the tree**
   (`tasks/T052-facts.md:100–101`). This draft treats it as satisfied by
   `HOST_SAVE_VERSION`/`CHECKPOINT_VERSION` and adds no bump (Reading 4).
   *Recommend:* state that explicitly in the final contract and do not invent
   a `SCHEMA_VERSION` here; a future breaking `World` shape is its own S-set
   task.
2. **`ScriptContinuation.locals` value type.** This draft mirrors `DataValue`
   as `ContinuationValue` to honour "persisted state never holds a `crpg-data`
   wire type" (T137, `crates/crpg-sim/AGENTS.md:515`). *Recommend:* keep the
   mirror (it decouples the sim's save from `DataValue` evolution); reusing
   `crpg_data::DataValue` is the simpler alternative if the architect judges
   `DataValue` not a "wire type".
3. **`TriggerContext` payloads.** The exact entity/quest fields per trigger
   kind are the interpreter's needs, drafted here as a faithful mirror of
   `Trigger` (`crates/crpg-data/src/ir.rs:44`) plus a subject. *Recommend:*
   confirm the payload set with the T053 draft; T052 only promises to persist
   and hash whatever `TriggerContext` holds.
4. **Nested `CallGraph` resume.** A `Wait` inside a called graph needs a call
   stack (depth ≤ 32). This draft stores only the top `graph`/`node`.
   *Recommend:* add a `frames: Vec<CallFrame>` field (`CallFrame { graph: Ulid,
   node: Ulid }`) if T053 requires it; it is additive and absence-skip, so it
   does not reopen the schema question.
5. **`Wait(0)` resume timing.** This draft makes `remaining == 0` "due
   immediately" (B-4); whether a `Wait(0)` resumes in the same interpreter
   pass or the next is the interpreter's. *Recommend:* leave it to T053, which
   owns dispatch order.
6. **`AreaError::ScriptContinuation` naming.** *Recommend:* this name; an
   alternative is `ContinuationOnEntity`. Both are additive last variants.

## Agent log

- 2026-11-19 (UTC) · opencode-go/deepseek-v4-pro + T052 draft a · Wrote the blind draft of the T052 contract: world-owned continuation, dialogue and quest state with an absence-skip serialized shape, a stage-2 wait countdown, structural load validation, and sim controllers for `crpg-script` to write, keeping the continuation as data in `World` with execution upward.
