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

- `World struct` (`crates/crpg-sim/src/world.rs:70`) — one loaded area's state: `entities: GenerationalArena<EntityMeta>`, `transforms`, `timeline`, `events: EventQueue<SimEvent>`, `rng`, `tick` (`crates/crpg-sim/src/world.rs:71`–`76`).
- `Combat fields` (`crates/crpg-sim/src/world.rs:84`) — `combatants: ComponentStore<Combatant>`, `combat: Option<CombatState>` (`:87`), `interners: Interners` (`:93`), all skipped at their absence value.
- `Area field` (`crates/crpg-sim/src/world.rs:100`) — `area: Option<Ulid>`, immutable, omitted when `None`.
- `Creature field` (`crates/crpg-sim/src/world.rs:105`) — `creatures: ComponentStore<CreatureState>` (T139), skipped when empty.
- `Movement fields` (`crates/crpg-sim/src/world.rs:110`) — `movers: ComponentStore<Mover>` (`:110`) and `nav: Option<NavRevision>` (`:114`), both skipped at their absence value.
- `Field order` (`crates/crpg-sim/src/world.rs:114`) — `creatures`, then `movers`, then `nav` are the last three fields; every optional/empty state is an absence compatibility shape, "not a hash exclusion" (`crates/crpg-sim/src/world.rs:102`–`108`).
- `Serde shape` (`crates/crpg-sim/src/world.rs:69`) — `World` derives `Serialize`; deserialization goes through `WorldRepr` in the same field order (`crates/crpg-sim/src/world.rs:126`) with `#[serde(default = "ComponentStore::new")]` on each store (`crates/crpg-sim/src/world.rs:134`).
- `Load validation` (`crates/crpg-sim/src/world.rs:150`) — `impl Deserialize for World` deserializes `WorldRepr`, then checks live ids and cross-field invariants before constructing a `World`.
- `World::new` (`crates/crpg-sim/src/world.rs:940`) — constructs the empty stores; `spawn` is at `:1001`, `despawn` at `:1021`, the `tick` getter at `:1136`.
- `Store getters` (`crates/crpg-sim/src/world.rs:1207`) — `creatures()`, `movers()` (`:1217`), plus crate-private mutable accessors (`crates/crpg-sim/src/world.rs:1202`, `crates/crpg-sim/src/world.rs:1212`, `crates/crpg-sim/src/world.rs:1223`).
- `Last field rule` (`tasks/T139.md:295`) — "Each new field is the **last** field of its struct, and each new variant comes after the existing ones"; the T139 package touched "the `World` struct, serde shapes and the meaning of a release" (`tasks/T139.md:59`).
- `T139 field mechanics` (`tasks/T139.md:568`) — `World` gains `creatures` as the last field after `area`, `WorldRepr` gains the matching defaulted entry, `despawn_tracked` gains `self.creatures.remove(id)` (`tasks/T139.md:584`), and `transfer_entity` copies the record (`tasks/T139.md:589`).
- `Architecture, creature store` (`docs/architecture/crpg-sim.md:387`) — `World.creatures` is a `ComponentStore<CreatureState>`, the last field after `area`, skipped when empty.
- `Architecture, movement fields` (`docs/architecture/crpg-sim.md:407`) — `World.movers` and `World.nav` are the last two fields after `creatures`, each skipped when empty.
- `Tick counter first` (`crates/crpg-sim/src/tick.rs:23`) — `tick` calls `tick_with_movement`, which calls `tick_tracked`; `world.advance_tick()` runs before the systems (`crates/crpg-sim/src/tick.rs:42`).
- `Advance tick` (`crates/crpg-sim/src/world.rs:1148`) — `advance_tick` advances the counter (saturating) before systems observe it.
- `System list` (`crates/crpg-sim/src/tick.rs:64`) — `run_systems` is hand-written: `encounter_clock_system` (stage 2), `timeline_system` (stage 3), `movement_system` (stage 4).
- `end_turn` (`crates/crpg-sim/src/tick.rs:55`) — pops the ascending `(key, id)` timeline head; no re-queue, no counter advance, no systems.
- `state_hash` (`crates/crpg-sim/src/hash.rs:37`) — BLAKE3 over canonical JSON produced by the serde impls; non-finite floats panic before serialization (`crates/crpg-sim/src/hash.rs:40`), and there are no hash exclusions (`crates/crpg-sim/src/hash.rs:10`).
- `S-set` (`docs/IMPLEMENTATION_PLAN.md:93`) — the serialized set is `crpg-contracts`, "the `World` struct or tick order in `crpg-sim`", schema versions/migrations, the GDExtension boundary and workspace CI files; packages marked **S** in §3 are in it (`docs/IMPLEMENTATION_PLAN.md:98`).
- `Tick order is serial` (`crates/crpg-sim/AGENTS.md:355`) — `run_systems` is a hand-written spec §10 stage list; appending is routine, reordering is a reviewed behaviour change, no scheduler or parallelism.
- `Stage order as built` (`crates/crpg-sim/AGENTS.md:609`) — `run_systems` is `encounter_clock_system`, `timeline_system`, then `movement_system` (stage 4); reordering or adding a stage is reviewed.
- `T052 S row` (`docs/IMPLEMENTATION_PLAN.md:282`) — T052 is marked `C → I, S` and its note is "`World` touch".

## Saving World today

- `Persist scope` (`crates/crpg-persist/src/lib.rs:3`) — "a versioned, checksummed, `zstd`-compressed save envelope around opaque caller bytes", with the S001 input/decompressed caps and an atomic-replace file store (T038).
- `Payload neutrality` (`crates/crpg-persist/src/lib.rs:7`) — "The envelope never encodes or inspects what it carries. The unit of save ... belongs to the caller."
- `Envelope layout` (`crates/crpg-persist/src/envelope.rs:1`) — envelope v1: a fixed 68-byte header, then the raw payload or exactly one zstd frame.
- `Save format version` (`crates/crpg-persist/src/envelope.rs:17`) — `SAVE_FORMAT_VERSION: u16 = 1`, "the only format version this build writes or reads"; `SAVE_MAGIC` is `CRPGSAVE` (`crates/crpg-persist/src/envelope.rs:15`).
- `Envelope caps` (`crates/crpg-persist/src/envelope.rs:24`) — `MAX_PAYLOAD_BYTES` 16 MiB, `MAX_ENVELOPE_BYTES` 16,777,284 (`:26`), checked before parsing/decompression.
- `Decode precedence` (`crates/crpg-persist/src/envelope.rs:6`) — the header is checked "in a fixed first-failure-wins order"; the decompressed-size cap is enforced while zstd produces bytes.
- `Persist invariant 1` (`crates/crpg-persist/AGENTS.md:29`) — "The envelope never encodes or inspects a `World`, `HistoryWorld` or checkpoint, and the crate has no workspace path dependency."
- `Checkpoint scope` (`crates/crpg-server/src/checkpoint.rs:1`) — in-memory host checkpoint bytes (T022 §7): full authoritative wrapper (world, sim journal and sequence/ack state), capture journal and watermark, protocol selection, incarnation identity.
- `Checkpoint versions` (`crates/crpg-server/src/checkpoint.rs:54`) — `CHECKPOINT_VERSION: u32 = 1` and `CHECKPOINT_VERSION_HOST_RECORDS: u32 = 2` (`:56`); version 1 is written whenever no host record is retained (`:12`), and "a version-1 checkpoint is a version-2 checkpoint with no host record, so nothing is migrated" (`crates/crpg-server/src/checkpoint.rs:19`).
- `Checkpoint cap` (`crates/crpg-server/src/checkpoint.rs:58`) — `MAX_CHECKPOINT_BYTES = 16 * 1024 * 1024`, checked before any JSON parsing (`crates/crpg-server/src/checkpoint.rs:23`).
- `Host save checkpoint` (`crates/crpg-server/src/host.rs:5123`) — `Host::save_checkpoint() -> Result<Vec<u8>, CheckpointError>`.
- `Checkpoint loader` (`crates/crpg-server/src/checkpoint.rs:176`) — `load_checkpoint`; the reader entry point is at `:230` and is capped before parsing.
- `Host save adapter` (`crates/crpg-server/src/save.rs:1`) — a host save is a `crpg-persist` envelope of kind `HOSTCKPT` whose payload is a fixed-order identity header followed by the checkpoint bytes "**verbatim**".
- `Save header` (`crates/crpg-server/src/save.rs:9`) — 4-byte `save_version` (`HOST_SAVE_VERSION: u32 = 1`, `crates/crpg-server/src/save.rs:43`), 16-byte campaign id, campaign-version text and engine-version text (`ENGINE_VERSION`, `crates/crpg-server/src/save.rs:45`).
- `Identity matching` (`crates/crpg-server/src/save.rs:19`) — campaign id and version are trusted caller facts that must match on load; the engine version is recorded and returned, never enforced (`crates/crpg-server/src/save.rs:263`).
- `Save identity type` (`crates/crpg-server/src/save.rs:89`) — `SaveIdentity`; `LoadedSave` is at `:98`.
- `Save entry points` (`crates/crpg-server/src/save.rs:272`) — `save_host_file` validates identity, takes `host.save_checkpoint()`, encodes, then calls `crpg_persist::save_file`; `load_host_file` is at `:287`.
- `No SCHEMA_VERSION` (`crates/crpg-server/src/save.rs:43`) — a repository-wide grep for `SCHEMA_VERSION` finds no matches; the save-side versions are `SAVE_FORMAT_VERSION` (`crates/crpg-persist/src/envelope.rs:17`), `HOST_SAVE_VERSION` (`crates/crpg-server/src/save.rs:43`) and `CHECKPOINT_VERSION`/`CHECKPOINT_VERSION_HOST_RECORDS` (`crates/crpg-server/src/checkpoint.rs:54`).
- `Spec save versioning` (`docs/CRPG_ENGINE_SPEC.md:851`) — "Save files carry engine version, campaign id and version, and a schema version." (The spec says this; no `SCHEMA_VERSION` constant exists in the tree.)
- `Campaign-data migrations` (`crates/crpg-data/AGENTS.md:6`) — the semantic-validation layer and the T012a migration framework; `docs/architecture/crpg-data.md:91` says the `migrations` module holds one sorted fifteen-family registry with only `crpg.item` at version 2 behind an edge.
- `Migration boundary` (`crates/crpg-data/AGENTS.md:42`) — a migration edge is required only when absence means exactly the old default; this is campaign-document migration, not save-file migration.
- `CLI migrate` (`docs/architecture/crpg-cli.md:500`) — `crpgc migrate <campaign-root>` migrates campaign data; it is not a save-file migration.
- `Gate 10` (`docs/architecture/crpg-server.md:975`) — spec §15.4 step 10 is active as "file-backed host save/load continuation equivalence": a host driven through `ingest → pump` is saved, dropped, reloaded under a strictly greater incarnation, and must match an uninterrupted run in authority hash, capture journal, watermarks and decoded delivery.
- `Gate 10 code` (`crates/crpg-server/tests/host_save.rs:706`) — "Gate 10: file-backed continuation equivalence", cases built on `file_save_load_equivalence` (`crates/crpg-server/tests/host_save.rs:709`) and per-step `authority_hash` assertions (`crates/crpg-server/tests/host_save.rs:524`).
- `Gate 10 non-claims` (`docs/architecture/crpg-server.md:987`) — not crash durability, Windows directory durability, cross-target byte equality, or the spec §8 breadth "every fixture campaign".
- `Save determinism` (`crates/crpg-server/AGENTS.md:293`) — saves are deterministic per build and per target; never compare save bytes, digests or hashes across targets; gate 10 compares runs inside one process and adds no golden.

## Authored dialogue, quest and event data today

- `Dialogue` (`crates/crpg-data/src/document.rs:155`) — authored dialogue: `id`, `slug`, `name`, optional `_note`, `entry: Ulid`, `nodes: Vec<DialogueNode>`; every object is `#[serde(deny_unknown_fields)]` (`crates/crpg-data/src/document.rs:154`).
- `DialogueNode` (`crates/crpg-data/src/document.rs:176`) — `id: Ulid` plus `body: DialogueBody`.
- `DialogueBody` (`crates/crpg-data/src/document.rs:191`) — internally tagged `kind`: `NpcLine` (speaker, `text_key`, opaque `conditions`, `on_enter`, optional `next`), `PlayerChoice` (`text_key`, `conditions`, `on_select`, `next`), `Jump { target }`, `Link { target }`, `End`; its doc says "without presentation or execution semantics" (`crates/crpg-data/src/document.rs:184`).
- `Required nullable next` (`crates/crpg-data/src/document.rs:204`) — an absent `NpcLine.next` must be written as explicit `null` (`required_nullable`), not omitted.
- `Quest` (`crates/crpg-data/src/document.rs:239`) — authored quest state machine: `id`, `slug`, `name`, `entry: Ulid`, `states: Vec<QuestState>`.
- `QuestState` (`crates/crpg-data/src/document.rs:260`) — `id`, display `name`, `terminal: bool`, `on_enter`, `transitions: Vec<QuestTransition>`.
- `QuestTransition` (`crates/crpg-data/src/document.rs:277`) — opaque `condition: String` plus `target: Ulid`; conditions are not parsed here.
- `TriggersDocument` (`crates/crpg-data/src/document.rs:354`) — an area-owned trigger aggregate: `area: Ulid` and `graphs: Vec<EventGraph>`.
- `EventGraph` (`crates/crpg-data/src/ir.rs:13`) — `id`, `slug`, `name`, `_note`, `entry: Trigger`, `start: Ulid`, `nodes: Vec<Node>`, `edges: Vec<Edge>`, `locals: Vec<VarDecl>`.
- `IR is data-only` (`crates/crpg-data/src/ir.rs:1`) — "Serializable authored event IR; execution and scheduling are external."
- `Trigger` (`crates/crpg-data/src/ir.rs:44`) — `AreaEnter`, `AreaExit`, `Interact`, `DialogueNode`, `QuestStateChange`, `Timer { ticks: u64 }`, `CombatStart`, `CombatEnd`, `Death`, `ItemAcquired`, `Custom { id }`.
- `Node` (`crates/crpg-data/src/ir.rs:78`) — `id: Ulid` plus `body: NodeBody`.
- `NodeBody Wait` (`crates/crpg-data/src/ir.rs:117`) — `Wait { ticks: u64 }` ("Wait a relative tick count; no absolute scheduling occurs here"); the other variants are `Condition`, `Action`, `Branch`, `Sequence`, `CallScript`, `CallGraph` (`crates/crpg-data/src/ir.rs:92`).
- `Edge` (`crates/crpg-data/src/ir.rs:161`) — `from: Ulid`, `port: Port`, `to: Ulid`; endpoint validation is deferred to T011.
- `Port` (`crates/crpg-data/src/ir.rs:144`) — `Next`, `True`, `False`, `Case { index }`.
- `ActionCall` (`crates/crpg-data/src/ir.rs:175`) — symbolic `action_id: String` plus named tagged `args`; no function pointers or registry.
- `ActionSignature` (`crates/crpg-data/src/ir.rs:185`) — symbolic action id plus ordered `ActionParameter`s; declarations only.
- `Document variants` (`crates/crpg-data/src/document.rs:403`) — `crpg.dialogue/1` (`:420`), `crpg.quest/1` (`:423`), `crpg.graph/1` (`:429`), `crpg.triggers/1` (`:450`).
- `No continuation type` (`crates/crpg-data/src/ir.rs:116`) — no `ScriptContinuation` or saved-continuation type exists in `crpg-data` (none found); `Wait` carries only the relative tick count.
- `Dialogue reachability` (`crates/crpg-data/src/validation.rs:2180`) — one diagnostic per dialogue node unreachable from a valid entry; emitted at `crates/crpg-data/src/validation.rs:2233`.
- `Quest completion` (`crates/crpg-data/src/validation.rs:83`) — `QuestNoCompletionPath` "A quest has no reachable terminal state"; emitted at `crates/crpg-data/src/validation.rs:2281`.
- `Locale keys` (`crates/crpg-data/src/validation.rs:2009`) — every name and dialogue text key must exist in every locale table.
- `No campaign fixture` — no file under `campaigns/` uses `crpg.dialogue/1` or `crpg.quest/1` (none found; `campaigns/` holds only `combat_basic` and `combat_srd`).
- `Broken-reference fixtures` (`crates/crpg-data/tests/fixtures/broken_references/dialogue/broken.json:1`) — a dialogue fixture with a broken entry; the quest counterpart is `crates/crpg-data/tests/fixtures/broken_references/quests/broken.json:1`.
- `Planned fixtures` (`docs/CRPG_ENGINE_SPEC.md:1502`) — `dialogue_branching`, `quest_multistage` and `save_load_stress` are named as future fixture campaigns, not present today.

## Scripting today

- `Crate scope` (`crates/crpg-script/src/lib.rs:3`) — "trusted action bindings today; the Lua 5.4 sandbox, event IR interpreter and serializable continuations are later tasks."
- `Not implemented` (`crates/crpg-script/src/lib.rs:10`) — "No Lua runtime, graph interpreter, wait/yield, continuation, native loading or host integration is implemented here."
- `Public surface` (`crates/crpg-script/src/lib.rs:15`) — `ActionBinding`, `ActionBindings`, `ActionHandler`, `BindingError`, `HandlerError`, `InvocationContext`, `MAX_BINDING_PROPOSALS`.
- `Binding table` (`crates/crpg-script/src/bindings.rs:4`) — `ActionBindings` pairs every T029a declaration with exactly one trusted Rust handler, assembled once at startup by `ActionBindings::new` (`crates/crpg-script/src/bindings.rs:6`); authored input can only select a bound handler, never install code (`crates/crpg-script/src/bindings.rs:10`).
- `Dispatch` (`crates/crpg-script/src/bindings.rs:13`) — one invocation runs synchronously: T029a call validation, T028 actor validation, one handler call with a read-only context, the `MAX_BINDING_PROPOSALS` and actor-substitution checks, then every returned `CombatAction` proposal applied in order through `perform_action` on a staged `World` clone.
- `Transactional publish` (`crates/crpg-script/src/bindings.rs:18`) — the staged world is published only when every proposal succeeds; any rejection leaves entities, resources, RNG, events and turn state unchanged.
- `Handler signature` (`crates/crpg-script/src/bindings.rs:48`) — `ActionHandler` is a function pointer `fn(&InvocationContext, &ActionCall) -> Result<Vec<CombatAction>, HandlerError>`.
- `Read-only context` (`crates/crpg-script/src/bindings.rs:53`) — `InvocationContext` exposes a shared `&World` and the validated actor; `world()` is at `crates/crpg-script/src/bindings.rs:123` and `actor()` at `:128`. It offers no world mutator, RNG access or host capability (`crates/crpg-script/AGENTS.md:31`).
- `No wait` (`crates/crpg-script/src/bindings.rs:25`) — "There is no wait, yield or continuation in this slice"; a handler that would wait returns `HandlerError::UnsupportedWait` (`crates/crpg-script/src/bindings.rs:181`), "a real error, never a saved continuation."
- `Immutable table` (`crates/crpg-script/src/bindings.rs:155`) — no mutator, late registration, hot replacement or deserialization; `ActionBindings::new` is at `:323`, `dispatch` at `:409`, the staged clone at `:441` and the single publish at `:448`.
- `Proposal bound` (`crates/crpg-script/src/bindings.rs:40`) — `MAX_BINDING_PROPOSALS = 32` limits applied simulation operations, not handler work.
- `D17` (`tasks/POST-T018-DECISIONS.md:371`) — T029b is an immutable trusted binding store and synchronous dispatch slice with "No Lua, Wait, native loading or resumed coroutine in this task" (`tasks/POST-T018-DECISIONS.md:373`).
- `D17 continuations` (`tasks/POST-T018-DECISIONS.md:381`) — "Later graph/interpreter tasks use world-owned serializable IR continuations in a separate earlier sim task. Never store live Lua stack/coroutine in a save."
- `D17 budgets` (`tasks/POST-T018-DECISIONS.md:383`) — 1024 node dispatches and 100,000 execution units per resumed trigger slice, nested depth 32; a `Wait` "commits a valid slice and serializes continuation; resume receives a new slice budget" (`tasks/POST-T018-DECISIONS.md:389`).
- `D21` (`tasks/DECISIONS-2026-09-30.md:51`) — `crpg-script` may take `crpg-core`, `crpg-data` and `crpg-sim` plus dev-only `serde_json`; no third-party runtime, no Lua.
- `ADR-0005 decision` (`docs/adr/0005-lua-sandbox-spike.md:96`) — "Go": the sandbox construction held against ten escape attempts, the instruction budget bounded an infinite loop, and the memory ceiling held.
- `ADR-0005 carry-forward` (`docs/adr/0005-lua-sandbox-spike.md:104`) — when `crpg-script` is built, carry forward `sandbox.rs`'s structure, including the explicit stripping of `load`/`loadfile`/`dofile` (`docs/adr/0005-lua-sandbox-spike.md:108`).
- `ADR-0005 coroutines` (`docs/adr/0005-lua-sandbox-spike.md:89`) — the coroutine library is never loaded, "so there is nothing to suspend a `Wait` with in the first place".
- `ADR-0005 gap` (`docs/adr/0005-lua-sandbox-spike.md:118`) — "Not exercised here: the async/`Wait`-node interaction"; re-verify once `crpg-script`/the event IR exist (`docs/adr/0005-lua-sandbox-spike.md:123`).
- `Script architecture` (`docs/architecture/crpg-script.md:43`) — `dispatch(world, actor, identity, call)` is the fixed pipeline; the context carries `&World` and the actor and "cannot reach `&mut World`, RNG, stores or host capability" (`docs/architecture/crpg-script.md:51`).
- `Legacy World only` (`docs/architecture/crpg-script.md:55`) — dispatch targets the legacy `World`, not `HistoryWorld`; a history-aware consumer needs its own sim batch boundary.
- `Saving semantics` (`docs/architecture/crpg-script.md:75`) — "Saving means saving the `World` between completed invocations and rebuilding identical bindings at startup; no table or partial call is persisted."
- `Execution needs a turn` (`crates/crpg-script/AGENTS.md:28`) — dispatch validates `validate_action(EndTurn { actor })`; a handler cannot run outside an active turn.

## Determinism rules that bind it

- `Lint scope` (`tools/lint/determinism.py:4`) — scans every `.rs` file under `crates/crpg-core/`, `crpg-rules/`, `crpg-sim/`, `crpg-nav/` and `crpg-ai/`; a missing linted crate is a `missing-crate` violation (`tools/lint/determinism.py:45`).
- `Both-scope rules` (`tools/lint/determinism.py:8`) — `HashMap`/`HashSet` iteration, `SystemTime`/`Instant`/`std::time::`, `std::thread`, and `rand::`/`thread_rng` fail in all five crates; the regexes are at `tools/lint/determinism.py:73`–`77`.
- `Sim float rule` (`tools/lint/determinism.py:19`) — `crpg-sim` bans `f64` only (`no-f64`, regex at `tools/lint/determinism.py:86`); `f32` spatial positions stay allowed and unsuffixed literals infer at `f32` sites.
- `Rule application` (`tools/lint/determinism.py:272`) — `crpg-sim` gets `("both", "no-f64-crates")`; `crpg-core`, `crpg-rules`, `crpg-nav` and `crpg-ai` get the full `no-float-crates` set (`tools/lint/determinism.py:69`).
- `Doctests are code` (`tools/lint/determinism.py:27`) — compiled doctest fences inside `///`/`//!` are scanned; fences tagged `text` or `ignore` are not (`tools/lint/determinism.py:111`).
- `Escape` (`tools/lint/determinism.py:106`) — a line ending `// determinism-ok: <reason>` is skipped, and an empty reason is itself a violation.
- `Root hash ban` (`AGENTS.md:43`) — no `HashMap`/`HashSet` in `crpg-core`, `crpg-rules`, `crpg-sim` or `crpg-ai`; use `IndexMap`/`BTreeMap`.
- `Root float ban` (`AGENTS.md:45`) — no `f32`/`f64` in `crpg-core`, `crpg-rules` or `crpg-ai`; `crpg-sim` may use `f32` for spatial positions only, never in a rules path, and `f64` is banned in `crpg-sim` too (`AGENTS.md:47`).
- `Bans cover tests` (`AGENTS.md:49`) — both bans apply to doctests and to `tests/`/`#[cfg(test)]` modules.
- `Replay determinism` (`AGENTS.md:20`) — exact-build, not cross-platform lockstep: Windows and Linux compare their own target-scoped goldens with the pinned toolchain and default features, never each other's hashes; no tolerance or missing-baseline skips.
- `ADR-0009 promise` (`docs/adr/0009-determinism-scope.md:20`) — same binary + same inputs ⇒ same hash sequence, tick for tick; cross-platform and cross-build sameness are explicitly not promised (`docs/adr/0009-determinism-scope.md:26`).
- `Golden scoping` (`docs/adr/0009-determinism-scope.md:33`) — golden files carry toolchain, platform and profile in the filename; CI compares goldens on canonical Linux only.
- `Hash exclusions` (`docs/adr/0009-determinism-scope.md:38`) — the exclusion list starts empty and an exclusion needs a test proving the field cannot affect behaviour; `state_hash` has no exclusions (`crates/crpg-sim/src/hash.rs:10`).
- `Sim invariant 5` (`crates/crpg-sim/AGENTS.md:205`) — `Transform` is `f32`-only for positions/velocities, is never rules input, and "No `f64` anywhere in the crate".
- `Sim invariant 8` (`crates/crpg-sim/AGENTS.md:220`) — "No `HashMap`/`HashSet` — anywhere, including tests. No clock, no threads, no I/O"; `DeterministicRng` with explicit stream names is the only randomness.
- `Hash panic` (`crates/crpg-sim/AGENTS.md:368`) — `state_hash` panics on non-finite floats by design; do not add a scrubbing pass.
- `crpg-sim goldens` (`crates/crpg-sim/tests/goldens/`) — `history_v1.x86_64-pc-windows-msvc.txt`, `history_v1.x86_64-unknown-linux-gnu.txt`, `history_v2.x86_64-pc-windows-msvc.txt`, `history_v2.x86_64-unknown-linux-gnu.txt`, `history_v3.x86_64-pc-windows-msvc.txt`, `history_v3.x86_64-unknown-linux-gnu.txt`, `movement_v1.x86_64-pc-windows-msvc.txt`, `movement_v1.x86_64-unknown-linux-gnu.txt`, `surface_v1.x86_64-pc-windows-msvc.txt`, `surface_v1.x86_64-unknown-linux-gnu.txt`.
- `history_v1 golden` (`crates/crpg-sim/tests/history_golden.rs:10`) — one `<step> <64-lowercase-hex-hash>` line per step from zero, final LF; target selection is compile-time and "a missing baseline fails, never passes".
- `history_v2 golden` (`crates/crpg-sim/tests/history_v2_golden.rs:9`) — the v2 wrapper's per-step hashes; `history_v3 golden` (`crates/crpg-sim/tests/history_v3_golden.rs:10`) is the v3 equivalent.
- `movement_v1 golden` (`crates/crpg-sim/tests/movement_golden.rs:10`) — the stage-4 movement schedule's per-step hashes; `surface_v1 golden` (`crates/crpg-sim/tests/surface_golden.rs:10`) is the T137 surface schedule's.
- `crpg-testkit goldens` (`crates/crpg-testkit/goldens/`) — `combat_basic_rust-1.98.0_x86_64-pc-windows-msvc_test-default.golden`, `combat_basic_rust-1.98.0_x86_64-unknown-linux-gnu_test-default.golden`, `replay_basic_rust-1.98.0_x86_64-pc-windows-msvc_test-default.golden`, `replay_basic_rust-1.98.0_x86_64-unknown-linux-gnu_test-default.golden`, `srd_lite_rust-1.98.0_x86_64-pc-windows-msvc_test-default.golden`, `srd_lite_rust-1.98.0_x86_64-unknown-linux-gnu_test-default.golden`.
- `Testkit golden readers` (`crates/crpg-testkit/tests/support/combat.rs:65`) — the combat_basic baselines; `replay_basic` is read at `crates/crpg-testkit/tests/support/mod.rs:36` and `srd_lite` at `crates/crpg-testkit/tests/support/srd.rs:68`.
- `S golden review` (`docs/IMPLEMENTATION_PLAN.md:415`) — the wave table schedules T052 with "Golden review for T052 if hashes move".

## Agent log

- 2026-11-19 (UTC) · opencode-go/deepseek-v4.1-flash + T052 facts · Recorded the binding requirements, World and save shapes, authored dialogue/quest/IR data, scripting surface and determinism goldens with checked `path:line` citations so the architect can write T052 without re-deriving them.
