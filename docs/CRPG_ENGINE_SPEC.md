# A Purpose-Built Open CRPG Engine — Technical Specification & Development Plan

**Status:** Draft 1, for review. Written to be usable as the initial technical specification by a human architect and by AI coding agents.
**Audience:** solo developer with AI coding agents, plus the agents themselves.
**Scope:** architecture, technology selection, repository and process design, roadmap, backlog.

---

## 0. Executive summary

### 0.1 The headline recommendation

**Do not fork Godot. Consume Godot as a replaceable presentation host, and build a Godot-free simulation core.**

Concretely:

```
crpg-core (Rust, no Godot)   ← rules, world state, simulation, campaign data,
                                scripting, AI, networking protocol, persistence
        │
        ├── crpg-server        headless binary. Links core. No graphics. Authoritative.
        │
        ├── crpg-client        Godot 4 application. GDExtension bridge to core.
        │                      Presentation + input + prediction of own movement only.
        │
        └── crpg-editor        Godot 4 application (NOT the Godot editor).
                               Custom UI. Edits campaign data via core's command API.
                               Connects to a running server as a privileged client.
```

Godot ships as a **pinned upstream tag plus a small patch queue**, consumed through GDExtension. Not a fork. If the patch queue exceeds roughly 5,000 lines or 20 touched files, that is a defect in the plan, not a milestone.

**Platform policy ([ADR-0012](adr/0012-windows-primary-platform.md)):**
`x86_64-pc-windows-msvc` is primary for development, product, release gating,
and behavioural baselines: client, editor, embedded single-player server,
dedicated server, and CLI. `x86_64-unknown-linux-gnu` is fully supported for
dedicated server, headless CLI/tooling, server-side extensibility, and
CI/testing. A platform-specific failure is a defect, not best effort. Linux
GUI client/editor builds are not promised; Linux headless support must not
depend on Godot. Other targets require an explicit support decision and
their own build/test scope.

### 0.2 Why this instead of a fork

Your own requirements argue against a deep fork more strongly than any general advice could:

1. You want a **headless authoritative server**. Godot's headless mode still carries SceneTree, ResourceLoader, servers, and the object system. Your simulation should not be able to accidentally depend on any of them. Physical separation is the only reliable enforcement.
2. You want **campaign data that is git-friendly, diffable, and AI-generatable**. Godot's `.tscn`/`.tres`/`ResourceLoader` are the wrong shape for that. So the "resource system" you listed as a *keep* is actually one of the first things to bypass.
3. You want a **radically simpler editor**. Godot's editor is not designed to be subsetted. `EditorNode` and its plugins are heavily interdependent. Deleting your way to a CRPG editor is more expensive and far more fragile than writing a new application on top of Godot's `Control`/theme system, which is genuinely excellent and which you get for free without touching engine source.
4. You want **AI agents to write most of the code**. A ~2M-line C++/SCons codebase is a poor substrate for agents: enormous context, 10–40 minute clean builds, and memory-safety failure modes that agents produce silently and often.
5. The one thing Godot gives you that is genuinely hard to replace is the **renderer, the asset import pipeline, and the animation system**. A deep fork is precisely the strategy that costs you upstream improvements to those systems.

The strategic payoff of the Godot-free core is **reversibility**. If Godot 5 breaks you, if the renderer disappoints, or if you decide Bevy or a custom wgpu renderer is better in year three, you replace one crate boundary. The rules, the campaign format, the netcode, the AI, and the server are untouched. That is worth more than any short-term velocity argument.

### 0.3 Honest timeline

A realistic estimate for a competent solo developer working consistently part-time with AI agents:

| Milestone | Estimate |
|---|---|
| Phase 0–2 (architecture, core skeleton, campaign format) | 6–10 weeks |
| First playable prototype (Section 19) | 6–9 months from zero |
| Editor that a non-programmer could use for a small module | 18–24 months |
| Usable PF2e subset (levels 1–5, ~150 spells, core combat) | 30–40 months |
| Product people compare favourably to the Aurora Toolset | 4–6 years |

If any part of this plan implies otherwise, the plan is wrong. Everything below is designed so that value accrues at each stage and abandonment at any phase leaves something usable.

---

## 1. Fork vs plugin vs clean-room: the real analysis

### 1.1 The three options as usually stated

**A. Godot plugin / GDExtension.** Ship a Godot project plus native extension. Users install Godot.
**B. Deep Godot fork.** Vendor the engine source, modify core, ship your own binaries.
**C. Clean-room.** Raylib/SDL/wgpu/Bevy, build everything.

These are the wrong three options, because they conflate two independent axes:

- **Axis 1: where does the simulation live?** Inside the engine's object model, or outside it?
- **Axis 2: how do you obtain the engine source?** Upstream binary, pinned source + patches, or hard fork?

Almost all the pain people attribute to "forking Godot" comes from Axis 1, not Axis 2. If your simulation lives inside `Node`/`Resource`/`SceneTree`, you are married to Godot regardless of whether you forked it. If your simulation lives in a standalone library, the engine is a rendering client and the fork question becomes almost boring.

So the real recommendation is **D: Godot-free core + pinned Godot + patch queue + custom applications built on GDExtension**. It is not a compromise between A, B and C. It dominates all three on the dimensions you care about.

### 1.2 Comparison

| Dimension | A: Plugin | B: Deep fork | C: Clean-room | **D: Core + pinned host** |
|---|---|---|---|---|
| Time to first playable | Fastest | Slow | Slowest | Fast (core dev parallel to client) |
| Technical risk | Low but capped | **High and compounding** | High | Medium, front-loaded |
| Control over UX | Poor (Godot editor visible) | Total | Total | **Total** (own binaries) |
| Editor development cost | Low, wrong shape | Very high (subtract from EditorNode) | Very high (build UI toolkit) | **Medium** (build on Control) |
| Renderer quality | Godot-grade | Godot-grade, drifting | You build it | **Godot-grade, current** |
| Networking suitability | Poor (ENet HLAPI is scene-oriented) | Must rewrite anyway | You build it | **Purpose-built, no legacy** |
| Long-term maintenance | Upstream churn breaks you | **Merge burden grows without bound** | You own 100% | Patch queue, bounded and measurable |
| Radical UX simplification | Impossible | Possible, expensive | Possible | **Possible, cheap** |
| Agent-friendliness | Medium (GDScript, weak types) | **Poor** (huge C++, slow builds) | Good | **Excellent** (small Rust crates) |
| Upstream merge burden | None | Severe | N/A | Small and CI-verified |
| Server performance | Bad (Godot headless overhead) | Bad unless stripped | Good | **Good** (no engine on server) |
| Debugging | Two runtimes, poor | C++ + build times | Good | Good (core testable in isolation) |
| Licensing | MIT, trivial | MIT, trivial | Depends | MIT, trivial |
| Community contributions | Godot devs can help | Fewer | Fewest | Split: Rust core + Godot shell |
| **Reversibility of the engine choice** | None | **None** | N/A | **High** |

### 1.3 What a "deep fork" actually costs

Concrete failure mode, drawn from projects that have tried it: you fork at Godot 4.4. You strip 2D, XR, and CSG. You modify `SceneTree` for your simulation. Eighteen months later Godot 4.7 lands with meaningful Vulkan and GI improvements, plus a `RenderingDevice` refactor. Your `SceneTree` changes now conflict with 400 files. You either spend two months merging, or you stop merging. If you stop merging, you have taken on maintenance of a renderer, an asset importer, and five platform backends, which is exactly the work you forked in order to avoid.

The fork is only correct if you need to modify **the parts of Godot you want to keep receiving updates for**. You do not. You need to modify the parts you can simply refuse to use.

### 1.4 When would I reverse this recommendation?

Be honest about the failure conditions:

- If GDExtension's API surface proves insufficient for a specific rendering need, you may need patches. That is fine. Patches are the plan.
- If `godot-rust` (gdext) becomes unmaintained, you would move the bridge to C++ GDExtension or C#. The core is unaffected.
- If you discover you want a *fundamentally different renderer* (large-scale terrain streaming, custom GI), consider replacing Godot with a wgpu renderer. Because the core is Godot-free, that is a client rewrite, not a project rewrite.

If, after a two-week spike (Task 1 in Section 24), GDExtension cannot render a skinned character driven by external state at acceptable cost, escalate to option C with Bevy as the host and keep everything else.

### 1.5 Licensing notes

- **Godot is MIT.** You may rebrand, ship your own binaries, and close-source your own code. You must reproduce the copyright notice. Ship a `Credits / Third-Party Licenses` screen in the client and editor, generated from a `licenses/` directory at build time. Make this a CI-checked step, not an afterthought.
- **Pathfinder 2e.** Paizo's ORC (Open RPG Creative) licence covers the Remaster rules content. Game *mechanics* are not copyrightable; specific *expression* is. Implementing ORC-licensed content requires reproducing the ORC notice and attribution. Do not ship Paizo trademarks, art, adventure text, or iconic character names outside what the Community Use Policy permits. Practical rule for this project: **the PF2e ruleset is a separately-licensed data package, versioned independently, with its own `LICENSE` and `NOTICE`.** The engine does not depend on it. This is a legal argument for the same architecture the engineering argues for.
- Get the ruleset packaging right in Phase 2, not Phase 11. Retrofitting licence boundaries into a monorepo is painful.

---

## 2. The recommended architecture

### 2.1 Process architecture

The planned product surface is three binaries plus one CLI, not a claim that
the current scaffolding ships them. Windows/MSVC supports all four; Linux/GNU
supports the headless dedicated server and CLI/tooling (ADR-0012):

| Binary | Contains | Renders? | Authoritative? |
|---|---|---|---|
| `crpg-server` | core, rules, sim, script VM, AI, net, persistence | No | **Yes** |
| `crpg-client` | Godot host + core in replica mode + net client | Yes | No |
| `crpg-editor` | Godot host + core in edit mode + privileged net client | Yes | No |
| `crpgc` (CLI) | validate / migrate / pack / run / replay / diff | No | n/a |

**Windows single-player embeds the same platform-neutral authoritative server
implementation in-process**, behind an in-memory `Transport` boundary.
Windows and Linux dedicated processes wrap that same implementation; a child
process on loopback remains a debugging-isolation option, not a separate
simulation. E012/E022 own the reusable host API/package decision, including
whether it is a library target in `crpg-server` or code in another existing
crate. This policy does not decide that placement or implement hosting.

There is no "single-player code path". This is the single most important structural decision after the Godot decision, and it must be enforced by making the client physically incapable of mutating authoritative state: the client's copy of the world is behind a `ReplicaWorld` type with no mutating methods except `apply_delta`. Prediction of own movement uses a buffer outside sim (bridge/client), never a mutable replica; `apply_delta` coverage is reserved here and defined fully in T018 (E015 decision 2026-09-06).

The authority boundary is unchanged when transport is in-memory. Keep
OS-specific process, filesystem, service, and presentation logic above
`crpg-core`, `crpg-rules`, and `crpg-sim`; no OS-specific branches belong in
those crates. This architectural constraint grants no permission for new
platform dependencies (ADR-0012).

### 2.2 Layer diagram

```
┌──────────────────────────────────────────────────────────────┐
│ Presentation (Godot 4)                                        │
│  client: scene proxies, cameras, UI, VFX, audio, input       │
│  editor: document UIs, viewport gizmos, graph editors        │
└───────────────┬──────────────────────────────────────────────┘
                │ GDExtension (godot-rust), narrow FFI surface
┌───────────────▼──────────────────────────────────────────────┐
│ crpg-client-bridge / crpg-edit                                │
│  replica world, interpolation, input encoding, command+undo  │
└───────────────┬──────────────────────────────────────────────┘
                │ pure Rust, no Godot types below this line
┌───────────────▼──────────────────────────────────────────────┐
│ crpg-net   protocol, codec, QUIC transport, interest mgmt    │
├──────────────────────────────────────────────────────────────┤
│ crpg-sim   world store, systems, tick, spatial queries,      │
│            movement, LOS, encounter/turn management          │
├──────────────────────────────────────────────────────────────┤
│ crpg-ai  │ crpg-script │ crpg-persist                        │
├──────────────────────────────────────────────────────────────┤
│ crpg-rules  stats, modifiers, effects, resolution, actions   │
├──────────────────────────────────────────────────────────────┤
│ crpg-data   campaign schema, serde, validation, migration    │
├──────────────────────────────────────────────────────────────┤
│ crpg-core   ids, Fx16_16, RNG, time, event substrate, errors │
└──────────────────────────────────────────────────────────────┘
```

Dependency direction is strictly downward. A CI lint enforces it (Section 17.4). Cycles are a build failure, not a code review comment.

### 2.3 Language selection

**Rust for everything below the presentation line. GDScript for presentation glue only.**

Why Rust rather than C++ or C#:

- **The compiler is a free code reviewer.** When agents write most of your code, a type system that rejects aliasing bugs, null derefs, and data races before the code runs is worth an enormous amount. This is the strongest single argument.
- **Crates are the parallel-agent boundary you asked for in Section 12 of your brief.** A Cargo workspace gives you compile-time-enforced module boundaries, per-crate test suites, per-crate public APIs, and per-crate ownership. You do not have to invent this.
- **`cargo test` with no setup.** Deterministic simulation tests, golden tests, and property tests are all first-class.
- **serde + schemars** gives you the campaign format, its JSON Schema, and its validation from one set of type definitions. This is a large chunk of Section 5 of your brief solved by a library.
- **No GC.** The server tick has no pause risk.
- Determinism is easier to enforce: you can lint against `HashMap` iteration and floating-point rules math.

Costs, stated honestly:

- Learning curve if you do not know Rust. Mitigation: the core is data-oriented, not lifetime-heavy; you will mostly write `struct`s, `enum`s, and functions over an arena. Avoid `async` in the sim entirely; confine it to `crpg-net`.
- `godot-rust` is a third-party binding. Mitigation: the FFI surface is deliberately small (Section 11.3), so replacing it is a bounded job.
- Fewer drive-by contributors than a GDScript project. Accept it. The editor UI in GDScript is where casual contributors can help.

**The main alternative worth respecting is C# with Godot .NET.** It is faster to write, has good tooling, and one language covers client and server. Reject it because: server GC pauses, weaker determinism story, .NET export friction on some platforms, and because the .NET runtime on the server pulls Godot's mono integration back into your headless build. If you personally cannot make progress in Rust after a genuine attempt, C# is an acceptable downgrade and the architecture survives unchanged.

**GDScript is explicitly rejected for gameplay and rules.** The server has no Godot. Any rule written in GDScript cannot run authoritatively. This is not a style preference; it is a structural impossibility.

### 2.4 The simulation substrate: ECS, scene graph, or hybrid

You asked for a reasoned decision, so here is the reasoning rather than the conclusion first.

**Requirements.**
- Hundreds to low thousands of entities per area, not hundreds of thousands.
- Turn-based and real-time-with-pause combat. Tick rate 10–20 Hz, not 120.
- **Replay determinism** is mandatory over an exact build (same binary, same inputs, same result). Cross-platform lockstep is *not* required because the server is authoritative (ADR-0009). ADR-0012 supersedes only ADR-0009 Decision 3's canonical-Linux-only selection: Windows/MSVC and Linux/GNU each reproduce an independently generated target-scoped golden under the pinned toolchain, normal test profile, and default features, never each other's hashes.
- Full state must serialize and deserialize losslessly, repeatedly, cheaply.
- Rules need to inspect arbitrary relational state: "all allies within 30 feet who are not frightened".
- Modders and AI agents need to add new component types from data.
- Agents must be able to read one system and understand it fully.

**Godot's SceneTree is rejected for simulation** because: it does not exist on the server; it is `Object`-based with reference counting and signals that make deterministic ordering awkward; serialization goes through `.tscn`; and node-per-entity costs are wrong for headless simulation.

**A full archetypal ECS (bevy_ecs, hecs) is rejected as the substrate** for less obvious reasons, and this is where most projects get it wrong:

- The performance argument does not apply at your entity counts. Cache-friendly archetype iteration matters at 100k entities. At 500 it is noise.
- Archetype migration on component add/remove invalidates pointers and complicates "one deterministic order of operations".
- Scheduler-driven parallel systems are a determinism hazard and a debugging hazard, and they are the main thing an ECS framework sells you.
- Serialization of a general ECS world with dynamic component registration is genuinely fiddly, and you will fight it every time you touch save/load.
- Agent comprehension: a system with an implicit query-based signature is harder for an agent to reason about than `fn apply_regeneration(world: &mut World, dt: Tick)`.

**Decision: a purpose-built entity/component store with explicit systems. Call it a "hybrid" if you like; it is really an ECS without the framework.**

```rust
// EntityId lives in crpg-core per ADR-0006 (E002 correction 2026-09-05:
// a single generational id; T007's arena is GenerationalArena<EntityMeta>).
pub struct World {
    entities: GenerationalArena<EntityMeta>,
    // Each component type is one dense store. Registered at compile time
    // for engine components, and via a typed dynamic store for ruleset components.
    transforms:  ComponentStore<Transform>,
    stats:       ComponentStore<StatBlock>,
    inventory:   ComponentStore<Inventory>,
    ai:          ComponentStore<AiState>,
    // ...
    dynamic:     DynamicComponentStore,  // ruleset/mod-defined, typed by schema
    spatial:     SpatialIndex,           // uniform grid, rebuilt per tick
    events:      EventQueue<SimEvent>,   // concrete queue in sim; core holds only the generic substrate (ADR-0008, E009 2026-09-06)
    timeline:    Timeline,              // ordered (InitiativeKey, EntityId); container in T007, advance rules in T008 (E015 2026-09-06)
    rng:         DeterministicRng,
    tick:        Tick,
}
```

Rules for this store, enforced by review and lint:

1. **Systems are ordinary functions** taking `&mut World` plus explicit parameters, called in a fixed, hand-written order by `fn tick(world: &mut World)`. No scheduler. No parallelism inside a single area's tick.
2. **No `HashMap` iteration in simulation code.** Use `IndexMap` or `BTreeMap`. Lint-enforced.
3. **No floating point in rules math.** Positions and velocities use `f32`. Anything a rule reads uses integers or fixed-point. Damage, modifiers, DCs, durations: integers.
4. **The entire `World` implements `Serialize`/`Deserialize`.** Saves are world snapshots. This is checked by a round-trip property test on every commit. (Skeleton only: the derive covers the world skeleton; every interned-handle boundary persists strings through the explicit `to_serializable`/`from_serializable` conversion owned by T014 per ADR-0006 Decision 4 — E014 clarification 2026-09-06.)
5. **Areas are simulated independently.** One `World` per loaded area. Cross-area effects go through a message queue on the `Campaign` object. This is your future scalability lever and it costs nothing now.

Godot's SceneTree is used on the client only, as a **presentation mirror**: each replicated entity gets a `Node3D` proxy created and destroyed by a single `SceneSyncSystem`. The proxy holds no gameplay state. If you ever find gameplay logic in a Godot node, that is a bug with a specific name: *authority leak*.

### 2.5 Time model

- Server tick: **20 Hz fixed** (50 ms). Configurable, but tested at 20.
- All durations in the rules are expressed in **rounds, turns, or ticks**, never seconds. A "6-second round" is a ruleset constant, not an engine one.
- Real-time-with-pause and turn-based are both expressed as a **Timeline**: an ordered queue of `(initiative_key, EntityId)`. In real time the timeline advances every tick; in turn-based it advances on `EndTurn`. One mechanism, two policies. Key type is `(InitiativeKey(i32), EntityId)` in a `BTreeMap` (deterministic order, `EntityId` tiebreak). The container is owned by T007; the advance policy — including the §6.2 turn-start vs §10 every-tick mapping — is owned by T008 (E015 decision 2026-09-06).
- The client renders at display refresh and interpolates between the last two received snapshots with a fixed ~100 ms delay buffer.

---

## 3. The rules kernel

### 3.1 The overengineering trap

The stated goal ("represent PF2e, D&D-likes, OSR, and original systems") invites a universal RPG metamodel. Every project that has attempted one has produced something either too abstract to author or too specific to reuse. The discipline that prevents this:

> **The kernel defines *how values are computed and how outcomes are decided*. It never defines *what the values mean*.**

The kernel therefore knows about seven concepts and nothing else:

1. **Stat** — a named value on an entity.
2. **Modifier** — a contribution to a stat or a roll, with a source, a type, a stacking rule, a condition, and a lifetime.
3. **Effect** — a data-defined package of modifiers and hooks with a lifecycle.
4. **Resolution** — a request to decide an outcome, and its result.
5. **Resource** — a pool with a maximum, a current value, and refresh triggers.
6. **Tag** — an interned string on entities, items, actions, damage, and effects.
7. **Event / Hook** — the point where a ruleset injects behaviour.

"Armour Class", "Strength", "Level", "Reaction", "Spell Slot", "Hit Points", "d20" are **ruleset data**. None appears in `crpg-rules`.

### 3.2 The primitives in detail

**Stats.** A `StatBlock` is `IndexMap<StatId, StatValue>` where `StatId` is an interned id declared by the ruleset, and `StatValue` is one of `Int(i32)`, `Fixed(Fx16_16)`, `Bool`, `Enum(EnumId)`, `Dice(DiceExpr)`, `Tags(TagSet)`. Persisted form is the explicit string-keyed conversion pair from ADR-0006 Decision 4, not a derive (E014 clarification 2026-09-06). Rulesets declare stats in data:

```json
{ "id": "hp", "kind": "int", "min": 0, "derived": null },
{ "id": "ac", "kind": "int", "derived": { "expr": "10 + @dex_mod + @armor_bonus" } }
```

**Modifier pipeline.** This is the highest-value primitive in the whole system, and the one you should build first and test hardest.

```rust
pub struct Modifier {
    pub source: SourceRef,          // effect, item, feat, script
    pub target: ModifierTarget,     // Stat(StatId) | Roll(RollTag) | Dc(...)
    pub op: ModOp,                  // Add(i32) | Multiply(Fx) | Set(i32) | Clamp{..}
    pub mod_type: ModTypeId,        // ruleset-defined: "status","circumstance","item"
    pub condition: Option<ConditionExpr>,
    pub priority: i16,
}
```

Resolution: gather applicable modifiers for `(entity, target, context)`, filter by `condition`, group by `mod_type`, apply the ruleset's **stacking policy** per group (PF2e: highest bonus and worst penalty per type; 5e: mostly no stacking of same-name; OSR: everything stacks), then apply ops in order `Set → Add → Multiply → Clamp`. Every query returns not just a number but a **ModifierBreakdown**, a list of contributions. Build the breakdown from day one. It powers the character sheet tooltip, the combat log, the editor's rules debugger, and every rules test you will ever write.

**Resolution.** One generic request type covers checks, saves, attacks, and flat checks:

```rust
pub struct ResolutionRequest {
    pub actor: EntityId,
    pub target: Option<EntityId>,
    pub roll: RollSpec,             // e.g. 1d20, 2d6, 3d6-drop-lowest, or "no roll"
    pub bonus_target: RollTag,      // what modifiers apply
    pub against: Against,           // Dc(i32) | Stat(EntityId, StatId) | Opposed(..)
    pub outcome_table: OutcomeTableId,
    pub tags: TagSet,
}
pub enum Outcome { CriticalSuccess, Success, Failure, CriticalFailure, Custom(u8) }
```

The `OutcomeTable` is ruleset data. PF2e's four bands (beat by 10 / meet / miss / miss by 10, natural 20 and 1 shifting one step) is a table. D&D 5e is a table with two bands plus a crit-on-natural-20 rule. A 2d6 Fighting-Fantasy style system is a table with two bands and no crits. **The engine never mentions d20.**

**Action economy as resource pools.** A `ResourcePool { id, max, current, refresh: RefreshTrigger }` where `RefreshTrigger` is `OnTurnStart | OnRoundStart | OnRest(RestId) | OnTick(n) | Never`. PF2e is one pool `actions` with max 3 refreshing on turn start, plus `reaction` max 1. 5e is `action`, `bonus_action`, `reaction`, `movement`. A real-time system uses per-ability cooldown pools refreshing on tick. Action *costs* are declared per-ability as `{ "actions": 2 }`. The engine checks affordability and deducts. It has no opinion about the numbers.

**Effects and conditions.** A `Condition` (frightened, prone, grappled) is just an `Effect` with a tag. An effect carries: modifiers, hooks, duration (`Rounds(n) | UntilEndOfTurn(EntityId) | Permanent | UntilRemoved`), stacking behaviour, a value (frightened 2), and lifecycle scripts. There is no separate `Condition` type in the kernel.

**Hooks.** The ruleset registers handlers on kernel events:

`BeforeRoll, AfterRoll, BeforeDamage, AfterDamage, OnApplyEffect, OnRemoveEffect, OnTurnStart, OnTurnEnd, OnRoundStart, OnMove, OnDeath, OnActionDeclared, OnEncounterStart, ...`

Handlers are pure-ish functions returning a list of **mutations** rather than mutating directly, so that ordering is explicit and the whole hook chain can be logged and replayed. This is what makes the rules debuggable.

### 3.3 The abstraction test (mandatory)

Before writing one line of PF2e, implement **two throwaway rulesets** entirely in data plus a few Lua handlers:

- `rulesets/minimal-d6/` — 3 attributes, 2d6 roll-under, no levels, no AC, damage as flat integers, no action economy (one action per turn).
- `rulesets/srd-lite/` — a d20 system with 6 attributes, AC, HP, levels, and a bonus action.

If either requires a change to `crpg-rules`, the abstraction is wrong and you fix it then, when it is cheap. If PF2e is your first ruleset, you will build PF2e's assumptions into the kernel without noticing. This is the single highest-leverage anti-overengineering *and* anti-underengineering test in the plan, and it costs about a week.

### 3.4 What the kernel deliberately does not do

- No character generator. Character creation is a ruleset-defined **wizard schema** interpreted by the client UI.
- No spell system. Spells are abilities with tags, resource costs, targeting, and effects.
- No class/level system. Progression is a ruleset-defined table of `(level, grants[])`.
- No encumbrance, crafting, or economy models. Items have stats and tags; rulesets do the rest.
- No balance validation. Not your job.

---

## 4. Campaign data format

### 4.1 Requirements ranked

Given that AI agents must generate and modify campaigns, and that campaigns must be diffable and collaborative, the ranking is:

1. Machine-generatable by an LLM without training on a bespoke syntax.
2. Diffable line-by-line in git, with stable ordering.
3. Schema-validated with precise, positioned error messages.
4. Versioned with a tested migration path.
5. Loadable incrementally by a server with no graphics.
6. Packable into a signed, content-addressed distribution artifact.

### 4.2 Format decision: canonical JSON + generated JSON Schema

**Source form: one document per file, one canonical JSON value per file.**

There are two document categories (E016, 2026-09-06). An **entity document**
contains one independently identified authored object. An **aggregate
document** contains the ordered collection or keyed table owned by one parent
scope; `placements.json`, `triggers.json`, locale tables, and
`variables/campaign_state.json` are the explicit initial aggregate documents.
An aggregate entry that can be referenced independently still carries a stable
object id. Adding another aggregate-document kind is a schema decision, not an
implicit exception to the entity-document rule.

Rejected alternatives and why:
- **RON**: elegant and Rust-native, but LLMs are markedly less fluent in it and tooling outside Rust is thin.
- **YAML**: diff-friendly but whitespace-fragile, and its type coercion rules (`no` → `false`, Norway problem) are a genuine source of silent campaign corruption.
- **TOML**: good for the manifest, poor for deeply nested data like dialogue trees.
- **Custom DSL**: costs you every tool you would otherwise get for free.
- **SQLite as source form**: kills git diffing and human editing. Correct for the *runtime* database on a persistent server, wrong for authoring.

JSON wins on the criteria that actually matter here. Its lack of comments is handled by a `"_note"` convention that the schema permits and the loader ignores.

**Canonicalisation is mandatory.** The editor writes files through a canonical writer: sorted keys except for ordered arrays, 2-space indent, `\n` endings, no trailing whitespace, arrays of objects one-per-line-group. `crpgc fmt` enforces it and CI checks it. Without this, every editor save produces a 400-line diff and collaboration dies.

**Schemas are generated from Rust types** via `schemars`, emitted to `schemas/` on build, and checked in. Agents and third-party tools consume them. A drift check in CI fails the build if the checked-in schema differs from the generated one.

### 4.3 Identity and references

Every authored object has:

```json
{
  "schema": "crpg.creature/3",
  "id": "01J8QK4W9YB6ZC3M0N7XKQ2R4A",
  "slug": "goblin-warrior",
  "name": "Goblin Warrior"
}
```

- `id` is an **object ULID**, generated once, never reused, never changed. All cross-references between authored objects use this `id`.
- `slug` is human-facing, unique within its type, and may be renamed freely.
- File paths are **irrelevant to identity**. Moving `creatures/goblin.json` to `creatures/humanoids/goblin.json` breaks nothing. This is what makes reorganisation and agent-driven refactors safe.
- The loader builds an index `id → (type, path)`. `crpgc validate` reports every dangling reference with the file and JSON pointer of the referrer.

Dependency coordinates occupy a separate identity domain. A **package id** is
an immutable, human-readable ASCII identifier matching
`[a-z0-9]+(?:[.-][a-z0-9]+)*`, such as `pf2e` or
`example.greenhollow`; it identifies a versioned campaign, module, or ruleset,
not an authored object. It is stable after publication and is not a renameable
object slug. Manifest dependency entries call this field `package` so package
coordinates cannot be mistaken for object ULIDs.

### 4.4 Layout

```
my-campaign/
  campaign.json              manifest: id, name, version, requires[], entry point
  campaign.lock              resolved package versions/checksums + assets.lock digest
  schemas/                   copies of the schemas this campaign validates against
  worlds/
    aurelia.json             world: metadata, area graph, global variables
  areas/
    greenhollow/
      area.json              area metadata, bounds, lighting, ambience, nav settings
      terrain.json           heightfield ref, splat refs, water planes
      placements.json        every placed instance: id, prefab ref, transform, overrides
      nav.json               navmesh bake settings (the baked navmesh is a build artifact)
      triggers.json
  creatures/    characters/    items/    abilities/
  effects/      factions/      dialogue/    quests/    encounters/
  loot/         shops/         cinematics/  audio/
  scripts/
    lua/*.lua                 advanced scripts
    graphs/*.json             visual event graphs (compile to the same IR)
  variables/
    campaign_state.json       declared variables, types, defaults, scopes
  assets/
    models/  textures/  audio/  fonts/
    assets.lock              path → blake3 hash → import settings
  locale/
    en.json                  string table; all display text is a key
  tests/
    smoke.replay             recorded input logs for regression
```

Notes on specific choices:

- **`placements.json` is separate from `area.json`** so that the high-churn file (object placement, edited constantly) is separate from the low-churn file (area settings). This matters for merge conflicts.
- **Localisation from day one, cheaply.** Every user-visible string is a key into `locale/en.json`. The editor writes the key and the English string simultaneously. Retrofitting this later is a multi-week rewrite of every editor form. Doing it now costs a helper function.
- **The navmesh is a build artifact**, not source. Same for lightmaps, imported textures, and compiled scripts. `build/` is gitignored.
- **`assets.lock` is the source-asset authority.** It records each source asset
  path, BLAKE3 content hash, and import settings so imports are reproducible.
  `campaign.lock` does not duplicate those entries: it owns exact resolved
  package versions and package checksums, plus one digest of `assets.lock`.
  The packaged `manifest.json` separately owns hashes of final packaged
  content. Each stage therefore has one hash authority.

### 4.5 Versioning and migration

- Every document carries `"schema": "<type>/<version>"`.
- Migrations live in `crpg-data/src/migrations/` as pure functions `fn v2_to_v3(doc: &mut Value) -> Result<()>`.
- The loader applies migrations in sequence, in memory. Files are only rewritten when the user saves.
- **Golden fixtures**: `crates/crpg-data/tests/fixtures/v1/…` through the current version, with expected post-migration output. A new schema version cannot merge without a migration and its golden test. This is a hard CI gate.
- Campaigns declare a minimum engine version; the engine refuses newer campaigns rather than half-loading them.

### 4.6 Dependencies and rulesets

`campaign.json`:

```json
{
  "schema": "crpg.campaign/1",
  "id": "01J...",
  "package": "example.greenhollow",
  "name": "The Greenhollow Incident",
  "version": "0.3.1",
  "engine": ">=0.4.0",
  "requires": [
    { "kind": "ruleset", "package": "pf2e",        "version": "^1.2" },
    { "kind": "module",  "package": "core-assets", "version": "^0.4" }
  ],
  "entry": { "world": "01J...", "area": "01J...", "spawn": "start" }
}
```

Resolution is deliberately primitive: flat list, semver ranges, single version per package id, error on conflict. No diamond resolution, no vendoring, no registry. `campaign.lock` pins exact package versions and checksums and records the `assets.lock` digest; it does not repeat per-asset hashes. Build a registry only if the project ever has users who need one.

### 4.7 Packaging and security

`crpgc pack` produces `name-0.3.1.crpg`, a zip containing:

```
manifest.json      id, version, engine req, dependency list, content hashes
content/           canonical JSON, exactly as in source
assets/            imported/compressed assets
signature          optional Ed25519 detached signature over manifest.json
```

Security posture:

- **The T1 campaign package contains no executable code except Lua scripts, which run in the authoritative server role only, in a sandbox** (Section 5.4). On Windows single-player that server is embedded on the player's machine; server-side is an authority boundary, not a promise of a remote host. A downloaded campaign cannot execute native code or client-side scripts.
- The client receives a **filtered subset**: presentation assets, UI definitions, and display text. It does not receive quest logic, hidden creature stats, trap locations, or DC values it should not know. Compute this filter server-side; do not rely on the client to ignore data it has.
- Hash-verify every asset against `manifest.json` at load. Reject on mismatch.
- Zip extraction must reject absolute paths, `..`, symlinks, and pathological compression ratios. Use a hardened extractor and write a test with a malicious archive fixture.

T1 campaign data and sandboxed Lua/ruleset content remain portable across
Windows/MSVC and Linux/GNU (ADR-0012). Platform-specific imported presentation
assets do not impose a Godot dependency on Linux headless loading. T0 native
extensions are separately operator-installed, target-specific artifacts, not
portable campaign payloads; their loading and packaging decision is deferred
to the open E023 decision in §12.1. No loader or ABI is specified here.

### 4.8 Making it AI-friendly

Specific affordances, since this is an explicit goal:

1. `crpgc schema <type>` prints the JSON Schema for one document type. An agent can fetch exactly the schema it needs without loading 200 KB of context.
2. `crpgc validate --json` emits machine-readable diagnostics with file, JSON pointer, error code, and a suggested fix. Agents iterate against it.
3. `crpgc new creature --slug goblin-warrior` scaffolds a valid minimal document. Agents start from valid, not from blank.
4. `crpgc explain <id>` prints an object plus its inbound and outbound references. Agents can navigate the graph without loading the campaign.
5. **Dialogue and quests have a text round-trip form** (Section 5.3). Agents author dialogue in an indented text syntax; `crpgc dialogue import/export` converts to and from the JSON graph losslessly. Writing dialogue trees as raw node-and-edge JSON is miserable for humans and error-prone for LLMs.

---

## 5. Events, visual scripting, and the script language

### 5.1 Four levels, one execution engine

The critical design decision: **the visual graph is a view over an intermediate representation, not a separate runtime.** Everything compiles to the same IR, and one interpreter runs it.

| Level | Who | Form | Compiles to |
|---|---|---|---|
| 0 | Anyone | Property checkboxes on a trigger/door/container | IR |
| 1 | Designers | Visual event graph | IR |
| 2 | Advanced designers | Lua event handlers | Lua (called from IR nodes) |
| 3 | Engine devs | Rust systems / native ruleset code | native, trusted |

An event graph node that says `Call Script("greenhollow/mayor_greeting")` is how level 1 and level 2 meet. A designer can start visual and drop to Lua for one node without rewriting anything.

### 5.2 The event IR

```
Graph := { entry: Trigger, nodes: [Node], edges: [Edge], locals: [VarDecl] }
Trigger := AreaEnter | AreaExit | Interact | DialogueNode | QuestStateChange
         | Timer | CombatStart | CombatEnd | Death | ItemAcquired | Custom(id)
Node    := Condition(expr)              -> true/false ports
         | Action(action_id, args)      -> next
         | Branch(expr, cases)          -> n ports
         | Sequence([Node])
         | Wait(ticks: u64)             -> next        (relative simulation ticks; suspends, persists)
         | CallScript(script_id, args)  -> next
         | CallGraph(graph_id, args)    -> next
```

Properties the IR must have:

- **Serializable mid-execution.** A `Wait` node inside a running graph must survive save/load and server restart. Model running graphs as entities with a `ScriptContinuation` component. Get this right early; retrofitting it is painful.
- **Tick-based waits.** `Wait` stores a relative count of simulation ticks,
  never seconds, rounds, or turns. Ticks exist in every simulation mode and
  resume exactly after save/load; a ruleset that exposes round- or turn-based
  waiting translates that concept into explicit rules scheduling.
- **Deterministic.** Node execution order is the edge order in the file. No implicit concurrency.
- **Budgeted.** A graph gets a maximum node count and instruction/bytecode budget per trigger invocation (deterministic; cf ADR-0005 — corrected from wall-clock per E008 2026-09-05, since a wall-clock abort diverges across machines). Exceeding it aborts the graph, logs an error with the campaign file and node id, and does not stall the tick.
- **Server-only.** Graphs execute on the server. The client receives their *effects*.

The `Action` vocabulary is registered by the engine and by rulesets: `StartDialogue`, `SetVariable`, `GiveItem`, `SpawnEncounter`, `AdvanceQuest`, `OpenDoor`, `PlayCinematic`, `ApplyEffect`, `MoveEntity`, `PlaySound`, `ShowMessage`, `TeleportParty`, and so on. Each action is a Rust function with a declared JSON-schema signature, so the editor can generate its property form automatically. **Do not hand-write editor UI per action.** Generate it from the signature. This is worth a day and saves months.

### 5.3 Dialogue

Dialogue is a specialised graph, not a general one, because dialogue has strong domain structure worth exploiting:

```
Node := NpcLine { speaker, text_key, conditions, on_enter_actions, animation, camera }
      | PlayerChoice { text_key, conditions, on_select_actions, check?, next }
      | Jump(node_id) | Link(node_id) | End
```

`check` on a player choice covers "[Diplomacy DC 18]" style gated options and resolves through the same `ResolutionRequest` pipeline as combat.

**Text round-trip form** (for humans and LLMs):

```
@node greeting
  NPC: dlg.mayor.greeting_1
  > choice_help    [if !quest.greenhollow.started]  "What's wrong?"
  > choice_leave                                    "Not interested."

@node choice_help
  NPC: dlg.mayor.explain
  ! quest.start greenhollow
  -> greeting
```

Lossless in both directions. Store the JSON as source of truth; the text form is an import/export path.

### 5.4 Script language: Lua 5.4 via `mlua`

Compared honestly:

| Option | Verdict |
|---|---|
| **GDScript** | **Disqualified.** Requires Godot; the server has none. |
| **C#** | Heavy runtime on the server, weak sandbox, poor hot-reload. No. |
| **Rhai** | Rust-native, safe, easy to embed. Slower, and LLMs write it poorly. Good fallback. |
| **Wren** | Elegant, small, fast. Tiny ecosystem, few LLM training examples. No. |
| **WASM (wasmtime)** | Best isolation, best for *untrusted server plugins*. Terrible authoring UX for campaign designers, heavy toolchain. **Adopt later for third-party server extensions only.** |
| **Custom language** | Absolutely not. This is a decade-long distraction. |
| **Lua 5.4 via `mlua`** | **Recommended.** |

Reasons for Lua: two decades of precedent as *the* game modding language, so your users may already know it; LLMs are extremely fluent in it; `mlua` gives you instruction-count hooks and memory limits for sandboxing; fast enough; small; hot-reloadable.

**The sandbox is not optional.** Campaign Lua runs in an environment with:

- No `io`, `os` (except a whitelisted `os.time` returning *sim* time), `require`, `dofile`, `loadstring`, `debug`, `package`, or raw FFI.
- `math.random` replaced by the deterministic sim RNG, seeded per-invocation from `(tick, entity, call_index)`.
- `pairs` replaced with a deterministic ordered iterator. This one line prevents an entire class of desync and replay bugs.
- An instruction-count hook that aborts after N instructions, and a memory ceiling per script context.
- No coroutine yields across tick boundaries; long waits use the IR's `Wait` node, which is serializable, not a suspended Lua coroutine, which is not.
- API surface exposed through a single generated binding module, so you can audit exactly what campaigns can touch.

Scripts are **event handlers with a bounded lifetime**, never long-running loops. Enforce this in the API design, not in documentation.

---

## 6. AI

### 6.1 Progression, simplest first

**Stage 1 — Reactive (MVP).** Aggro radius, line of sight, move to nearest hostile, use the first affordable ability. About 200 lines. Ship this.

**Stage 2 — Utility scoring (the real system).** For each actor turn, enumerate candidate `(ability, target, position)` tuples, score each with a weighted sum of considerations, pick the best. This is the correct architecture for CRPG combat and it is where you should stop for a long time.

```rust
struct Consideration { id: ConsiderationId, weight: Fx, curve: Curve }
// e.g. expected_damage, target_threat, self_risk, ally_in_aoe (negative),
//      resource_cost, distance, flanking_gained, cover_gained
```

Considerations are engine-provided; **weights are ruleset and creature data**, so a designer tunes a "cowardly goblin" versus a "berserker" without code:

```json
"ai": { "profile": "melee_aggressive",
        "weights": { "self_risk": -0.2, "expected_damage": 1.4 } }
```

**Stage 3 — Behaviour trees for non-combat.** Schedules (sleep, work, tavern), patrols, reactions to the player. Use a data-defined BT with a small node set. Do not use a BT for combat action selection; utility is better at it.

**Stage 4 — Tactical positioning.** Influence maps over the navmesh: threat, cover, AoE danger, chokepoints. Feed them as considerations into stage 2. This is where AI starts to feel intelligent, and it is also where cost explodes. Do it after the game is playable.

**Do not build GOAP.** Do not build a planner. Do not build learning. For a CRPG they buy little and cost enormously.

### 6.2 Two constraints that matter architecturally

1. **AI may only use the same action-legality API as a player.** `fn legal_actions(world, entity) -> Vec<ActionOption>` is shared. If the AI can do something a player cannot, that is a bug by construction. This also gives you player-party auto-resolve and "suggested action" hints for free.
2. **AI is server-side, budgeted, and time-sliced.** Combat AI computes on turn start with a node budget. Ambient AI (schedules, wandering) runs on a round-robin across ticks: at most N entities re-evaluate per tick. This keeps tick time bounded regardless of population.

### 6.3 Navigation

Godot's `NavigationServer` is unavailable on the server, so navigation is **replaced**, not kept.

- Bake navmeshes with **Recast** (via Rust bindings) in the editor, at build time, from area geometry. Store as a build artifact keyed by area id and a hash of the source geometry.
- Server pathfinding uses **Detour** or a Rust equivalent over the same navmesh. Server and client must use the identical navmesh file, or clients will predict movement the server rejects.
- Local avoidance: skip it initially. Party members bumping into each other is a known CRPG aesthetic. Add simple steering later if it grates.

---

## 7. Networking

### 7.1 First principles for this genre

A CRPG is not a shooter. The consequences are large and mostly liberating:

- **No rollback netcode. No lockstep.** Do not build them.
- Player-visible latency tolerance is roughly 150–250 ms for anything except own-character movement.
- Entity counts per client's interest set are in the hundreds.
- **Prediction is needed for exactly one thing: the local player's own movement.** Everything else is server-decided and displayed after the fact.
- Cheating matters because campaigns are shared and persistent worlds are a goal. Authority plus per-client data filtering handles it.

### 7.2 Transport: QUIC via `quinn`

Rejected: ENet (no encryption, C dependency, no stream multiplexing), raw UDP (you rebuild reliability, ordering, congestion control, and crypto), TCP alone (head-of-line blocking on movement), WebRTC (complex signalling for no benefit outside browsers).

QUIC gives you, in one dependency: encryption and authentication via TLS 1.3, reliable ordered streams *and* unreliable datagrams on one connection, per-stream flow control so a large area transfer does not block combat events, connection migration across network changes, and congestion control you did not write.

Channels:

| Channel | QUIC feature | Contents |
|---|---|---|
| `control` | reliable stream | handshake, auth, area load, save/load, admin |
| `events` | reliable ordered stream | sim events: damage, effects, dialogue, quest, spawns |
| `snapshot` | unreliable datagram | positions, orientations, animation state |
| `bulk` | separate reliable stream | campaign package transfer, asset delivery |

### 7.3 Protocol shape

**Server → client** is a stream of `WorldDelta` messages:

```rust
enum DeltaOp {
  EntityEnter { net_id, archetype, initial: FilteredComponents },
  EntityLeave { net_id },
  ComponentUpdate { net_id, component_id, payload },
  SimEvent(SimEvent),          // "Grigor took 7 slashing damage, critical"
  TimelineUpdate { .. },
  DialogueOpen { .. }, DialogueClose,
}
```

Sim events are the same event stream the combat log renders and the replay system records. One mechanism, three uses.

**Client → server** is a small, closed set of intents, never state:

```rust
enum ClientIntent {
  MoveTo { position, tick },
  DeclareAction { ability, targets, position },
  DialogueChoice { node_id, choice_index },
  UseItem { item }, Interact { net_id },
  EndTurn, RequestSave, Chat { .. },
}
```

There is no `SetPosition`, no `ApplyDamage`, no `SetVariable`. **If a message name is a verb the server should decide, it does not exist.** Every intent is validated against `legal_actions` before execution, with an explicit rate limit and payload size cap.

### 7.4 Interest management and the anti-cheat boundary

- **Tier 1 (now):** interest set = the area the player's character is in. Simple, correct, sufficient for 8 players in a 500×500 m area.
- **Tier 2 (designed for, not built):** grid-cell AOI within an area. `InterestSet` is an interface from day one, so replacing the implementation touches one file.
- **Per-client component filtering is mandatory, not optional.** A hidden creature must not be sent to a client that has not perceived it. A trap's existence must not be replicated until detected. A locked chest's contents must not be sent. This is computed server-side per client per tick. Skipping it means stealth, perception, and secrets are cheatable by anyone with a packet sniffer, and retrofitting it means auditing every component. Build `fn visible_to(world, viewer, entity) -> ComponentMask` in the first networking milestone.

### 7.5 Prediction, reconciliation, interpolation

- Client predicts own movement: applies input immediately, keeps a ring buffer of `(tick, input, predicted_position)`.
- Server sends authoritative position with the last-processed input tick. Client discards acknowledged inputs, and if the divergence exceeds a threshold, snaps or eases to the server position and replays unacknowledged inputs.
- All other entities are interpolated between the last two snapshots with a fixed ~100 ms buffer.
- **Nothing else is predicted.** An attack shows a wind-up animation and a "pending" state until the server resolves it. This is the correct trade for the genre and it removes an entire category of bugs.

### 7.6 Reconnection and persistence

- Session token issued at auth; on reconnect within a grace window the server re-attaches the player to their existing character rather than rejoining fresh.
- Resync is a full area snapshot, not a delta. Delta-since-disconnect is an optimisation for later.
- The character remains in the world during the grace window, controlled by party AI or frozen (campaign-configurable). Combat cannot be escaped by pulling the ethernet cable.

### 7.7 Deliberately deferred

Sharding, cross-server travel, a login/master server, matchmaking, NAT punching (require port forwarding or a relay initially), voice, and server clustering. The area-scoped `World` and the `InterestSet` interface are the only concessions to that future, and both are free.

---

## 8. Persistence

- **Trait `PersistenceBackend`** with two implementations.
- **`SnapshotBackend` (build now):** the entire `World` plus campaign state serialized with `postcard`, compressed with `zstd`, written atomically (temp file, fsync, rename). Save files carry engine version, campaign id and version, and a schema version. Single-player and small multiplayer use this.
- **`SqliteBackend` (build much later):** per-entity rows, dirty-tracking, incremental writes. Only when a persistent world with hundreds of characters demands it.
- **Save/load is exercised constantly, not occasionally.** A CI test loads every fixture campaign, runs 500 ticks, saves, loads, runs 500 more, and asserts the state hash matches an uninterrupted 1000-tick run. Save bugs found in month 30 are catastrophic; found in month 3 they are Tuesday.
- Autosave on area transition and on encounter end. Rolling slots. Never overwrite the only save.

---

## 9. Client architecture

### 9.1 Structure

```
crpg-client (Godot 4 application)
├── Rust (GDExtension, crpg-client-bridge)
│   ├── net client (quinn), delta application
│   ├── ReplicaWorld  (crpg-sim in read-only replica mode)
│   ├── interpolation buffer, own-movement prediction
│   ├── intent encoding + rate limiting
│   └── query API exposed to GDScript
└── GDScript / Godot scenes
    ├── SceneSync        entity → Node3D proxy lifecycle
    ├── CameraRig        isometric/over-shoulder/free, PF-style
    ├── UI               HUD, character sheet, inventory, dialogue,
    │                    combat log, timeline, spell book, journal
    ├── VFX / audio      driven by SimEvent stream
    └── Input            action map, click-to-move, hotbar, controller
```

### 9.2 Rules the client must obey

1. **No authoritative logic.** The client may compute *predictions* and *display hints* (estimated hit chance, movement range preview), but it must recompute nothing that determines an outcome. A prediction that disagrees with the server is a cosmetic bug; a client that decides is a security hole.
2. **The client renders `SimEvent`s, it does not infer them.** Do not diff HP to discover that damage happened. The server sends `Damage { target, amount, type, outcome, source }` and the client plays the number, the sound, and the flinch. This makes the combat log, the VFX, and the replay system the same system.
3. **UI reads the replica world through a stable query API**, not through Godot nodes. `world.stat(entity, "hp")`, `world.entities_in_area()`, `world.timeline()`. Godot nodes exist only for rendering and picking.
4. **Client-side scripting, if it ever exists, is UI-only** and cannot touch the replica world's authoritative fields.

### 9.3 Presentation data

Creatures, items, and effects reference **appearance definitions** kept separate from their rules data:

```json
// creatures/goblin-warrior.json
"appearance": "app.goblin_warrior"
// appearance/app.goblin_warrior.json
{ "model": "assets/models/goblin.glb", "scale": 0.8,
  "anim_set": "humanoid_small", "material_overrides": {...},
  "attachments": { "weapon_r": "socket_hand_r" } }
```

This separation is what lets the server load a campaign without knowing what a mesh is, and lets a total conversion reskin a ruleset without touching rules.

### 9.4 Animation

Use Godot's `AnimationTree` with a **fixed set of animation slots** (`idle`, `walk`, `run`, `attack_melee`, `attack_ranged`, `cast`, `hit`, `die`, `interact`) declared per `anim_set`. Content maps its animations into slots. The sim never names an animation; it emits events and states, and the client maps them.

---

## 10. Server architecture

This is one platform-neutral authoritative implementation, with Windows
in-process single-player hosting and Windows/Linux dedicated process adapters
(ADR-0012). The diagram separates dedicated startup from reusable host logic;
E012/E022 must decide its actual API/package allocation before implementation.
Neither embedded hosting nor editor Play grants the client direct mutation
access. OS-specific adapters stay above core/rules/sim, and all headless
server surfaces remain Godot-free.

```
crpg-server
├── main: config, campaign load, listener, admin console
├── net layer (quinn): sessions, auth, rate limits, per-client filtering
├── SimHost
│   ├── one Area instance per loaded area (each an isolated World)
│   ├── fixed 20 Hz tick loop per area (single-threaded per area;
│   │   areas may run on a thread pool — one area, one thread, no sharing)
│   └── cross-area message bus (party travel, global quest state, timers)
├── rules engine (crpg-rules) + loaded ruleset
├── script host (mlua) with per-invocation budgets
├── AI scheduler (budgeted, round-robin)
├── persistence backend
└── admin/RPC surface (used by the editor as a privileged client)
```

Tick order is fixed and written out in one function, because tick order is a rules-visible decision:

```
1. drain network intents, validate, enqueue
2. advance timers, effect durations, resource refreshes
3. resolve declared actions in timeline order
4. movement integration + collision + trigger volumes
5. AI decisions (budgeted)
6. run event graphs and script continuations
7. perception / visibility update
8. build per-client deltas, enqueue
9. persistence tick (autosave check)
```

The server has a `--headless-deterministic` mode: no network, inputs from a replay file, fixed seed, prints a state hash per N ticks. This is the backbone of the entire test strategy (Section 18).

---

## 11. Editor architecture and UX

### 11.1 The key architectural idea

**The editor is a privileged client of a running server.**

On the primary Windows target, press Play and the editor launches the same
authoritative server implementation in-process with the current campaign,
then attaches through in-memory transport as a client with GM privileges
(ADR-0012). It cannot mutate authoritative state directly. Linux editor GUI
support is not promised. This single decision gives you:

- Instant test-play from any point in the campaign, at almost no additional cost.
- Live editing of a running session (move an NPC, give an item, fire a trigger).
- Multiplayer testing, since the editor's server accepts real clients.
- A GM/DM mode for tabletop-style play, essentially for free, later.
- Debugging tools that work identically against a local test server and a remote production one.

Godot's own editor works this way (in-editor play), and this is the one editor idea worth borrowing.

### 11.2 Document model, undo, and validation live in Rust

```rust
// crpg-edit
pub struct CampaignDocument { /* loaded, indexed campaign */ }
pub enum EditCommand {
    SetField { object: Id, pointer: JsonPointer, value: Value },
    CreateObject { kind, template }, DeleteObject { id },
    PlaceInstance { area, prefab, transform }, MoveInstance { .. },
    AddDialogueNode { .. }, ConnectGraphNodes { .. }, /* ... */
}
impl CampaignDocument {
    pub fn apply(&mut self, cmd: EditCommand) -> Result<CommandReceipt>;
    pub fn undo(&mut self) -> Result<()>;
    pub fn redo(&mut self) -> Result<()>;
    pub fn validate(&self) -> Vec<Diagnostic>;
    pub fn save(&self) -> Result<Vec<PathBuf>>;
}
```

Consequences worth stating plainly:

- **Undo/redo is one implementation**, not one per editor panel. Every mutation goes through `apply`.
- **AI agents drive the same API headlessly.** `crpgc apply commands.json` performs exactly what the GUI performs, including validation. Agent-generated content cannot bypass invariants the GUI enforces.
- The GDScript UI is a **view**. It renders document state and emits commands. When a command is applied, the document emits a change notification and views refresh. Keep GDScript logic-free enough that a bug there cannot corrupt a campaign.

### 11.3 The FFI surface (keep it small)

Between Rust and GDScript there are roughly five object types:

`Document` (open/save/apply/undo/validate/query), `Session` (connect/play/intent/query replica), `Assets` (import/scan/thumbnail), `Diagnostics` (list/subscribe), `Preview` (bake nav, compute LOS, ranges).

Everything else stays in Rust. A small FFI surface is what makes the "swap Godot for something else" escape hatch real.

### 11.4 UX design

**Main window.** Three regions, IDE-like, with none of Godot's vocabulary anywhere.

```
┌───────────────────────────────────────────────────────────────────────┐
│ Campaign ▾  Edit ▾  World ▾  Rules ▾  Test ▾  Package ▾    ▶ Play  ⏹  │
├────────────┬────────────────────────────────────────┬─────────────────┤
│ CAMPAIGN   │                                        │ PROPERTIES      │
│ ▾ Worlds   │            DOCUMENT AREA               │                 │
│   ▾ Aurelia│      (tabs; each tab is a typed        │ generated from  │
│     Green… │       editor for one document)         │ schema; never   │
│ ▾ Creatures│                                        │ hand-written    │
│ ▾ Items    │                                        │                 │
│ ▾ Dialogue │                                        ├─────────────────┤
│ ▾ Quests   │                                        │ REFERENCES      │
│ ▾ Factions │                                        │ used by 4 …     │
│ ▾ Scripts  │                                        │ uses 7 …        │
├────────────┴────────────────────────────────────────┴─────────────────┤
│ PROBLEMS (3)  │ CONSOLE  │ PLAYTEST LOG  │ SEARCH                     │
│ ⚠ dialogue/mayor.json #/nodes/4: choice leads to deleted node          │
│ ⚠ quests/greenhollow.json: no reachable completion path               │
│ ✖ areas/greenhollow/placements.json #/12: creature id not found       │
└───────────────────────────────────────────────────────────────────────┘
```

Three UX commitments that distinguish this from every hobby editor:

1. **A continuous Problems panel.** The campaign is validated on every change, in the background, like a compiler. Dangling references, unreachable dialogue, quests with no completion path, encounters referencing deleted creatures, orphaned assets, missing localisation keys. This is the feature that will make people prefer your editor to the Aurora Toolset.
2. **The References panel on every object.** "What uses this?" and "What does this use?" Deletion is safe because you can see the blast radius, and the editor offers to fix or block.
3. **Property forms are generated from schemas.** Adding a field to a creature adds it to the editor with zero UI work. Hand-written forms are how editors rot.

**Per-editor notes.**

- **World editor.** Node graph of areas with connections (doors, portals, world-map travel), plus a world-map image with pinned locations. Not a 3D view.
- **Area editor.** The main 3D viewport. Terrain sculpt/paint (heightfield, splat), object placement with grid/snap/scatter brushes, walkmesh preview, trigger volume drawing, light placement, spawn points, encounter regions. Layers with visibility toggles: terrain, statics, creatures, triggers, nav, lighting. A one-key **walkable overlay** and a one-key **line-of-sight probe** from the cursor.
- **Creature / character editor.** Left: identity, appearance, faction, AI profile, loot table, dialogue, scripts. Right: a live rules panel showing the ruleset-defined stat block with the full modifier breakdown for every derived value. Templates and inheritance (`base: "goblin"` with overrides) so a variant is five lines, not a copy.
- **Item / weapon / armour editor.** One item editor. "Weapon" and "armour" are ruleset-defined item categories that add schema-driven fields, not separate editors. Resisting the urge to build three editors here is a real test of whether the ruleset abstraction is working.
- **Dialogue editor.** Node graph as primary view. A **text view toggle** with the round-trip syntax from Section 5.3. Speaker colour-coding, condition and action badges on nodes, a "walk this conversation" simulator that runs the real dialogue engine against a chosen test character. Unreachable nodes greyed out live.
- **Quest editor.** States and transitions, with each transition showing what triggers it and what it updates. Journal text per state. A reachability analysis that shows which states can actually be reached from the start.
- **Encounter editor.** Creature list with counts, spawn points, spawn conditions, difficulty estimate from the ruleset (a ruleset-provided function, not an engine one), and a **dry-run simulator** that runs the encounter headlessly 100 times against a reference party and reports win rate and average rounds. This is a genuinely novel feature and it falls out of the deterministic headless server for almost free.
- **Rules editor.** Browse and override ruleset definitions for this campaign: stats, conditions, outcome tables, damage types, progression tables. Campaign-local overrides are diffs against the ruleset, stored in `rules/overrides.json`.
- **Event graph editor.** Node graph over the IR from Section 5.2. Node property forms generated from action signatures. Live highlighting of the executing node when attached to a running session.
- **Script editor.** Adequate, not ambitious. Syntax highlighting, LSP-backed completion for the exposed API (ship a generated Lua definition file), error markers from the sandbox. Do not build a debugger; add print-to-console and breakpoint-on-error, and let people use external editors.
- **Campaign debugger.** Attached to a running session: entity inspector, variable watch, quest state, active effects with sources, the modifier breakdown for any value, the event log with filtering, a rules trace showing every roll and its inputs, and time controls (pause, step one tick, step one round).
- **Multiplayer testing.** "Launch N clients" spawns real client processes against the editor's server, tiled on screen, with a network condition simulator (latency, jitter, loss) in front of them. Building this in Phase 10 rather than Phase 13 is the difference between multiplayer that works and multiplayer that is theoretically supported.
- **Packaging.** One dialog: validate, select supported target artifacts, choose whether to bundle the portable ruleset content, sign, and export `.crpg` plus optional Windows/Linux dedicated-server config. T1 data and sandboxed Lua remain portable; T0 native artifacts are target-specific and their loading/packaging decision is deferred (§12.1, ADR-0012). Refuse to package with unresolved errors; this dialog does not promise Linux GUI builds.

---

## 12. Modding and the security model

### 12.1 Three trust levels, three enforcement mechanisms

| Tier | Examples | Trust | Enforcement |
|---|---|---|---|
| **T0 Native** | engine, rulesets shipped as Rust, server plugins | Full | Operator installs them deliberately. Not sandboxed. Signed and listed in the server config. |
| **T1 Campaign content** | campaign JSON, Lua scripts, event graphs, assets | Semi | Runs **server-side only**, in the Lua sandbox, with instruction/memory/time budgets and a whitelisted API. |
| **T2 Client-side** | UI themes, HUD layouts, model replacements, sounds | **None** | Never executes code. Data only. Never affects sim. Server-verified where it could confer advantage. |

The dangerous idea to reject explicitly: **do not let clients download and run campaign scripts.** If a player connects to a server, they receive data and events, never logic. This keeps the "join a stranger's server" case safe, which is the one that would otherwise sink the project's reputation.

Here "server-side" denotes the authority role, including the Windows embedded
single-player server, not Linux or a necessarily remote machine. T1 campaign
data and sandboxed Lua/ruleset content are portable; T0 native extensions need
separate Windows/MSVC and Linux/GNU artifacts (ADR-0012). Linux server-side
extensibility is fully supported, and must not depend on Godot.

**Open planning-only decision:**
[E023: native-extension loading and packaging](../tasks/E023-native-extension-loading-and-packaging.md)
tracks T0 native loading and packaging across both supported targets. It must
reconcile target artifacts and ABI/loading with the rule that only
`crpg-godot` may use `unsafe`. No loader, dependency, unsafe exception, or
stable ABI claim is authorized by T009c; the mechanism remains separately
decided. The planning task exists; no loader or packaging implementation is
claimed.

### 12.2 Ruleset modding

Rulesets are packages with the same structure as campaigns: `ruleset.json` manifest, data documents, Lua hooks, optionally a native Rust crate (T0). A ruleset declares which stats, conditions, damage types, outcome tables, and actions it provides. A campaign may override any ruleset document locally.

Data and sandboxed Lua portability does not make an optional T0 native crate
a portable binary. Operators need the artifact for their target; the future
§12.1 decision owns its delivery/loading mechanism (ADR-0012).

Total conversions are the acceptance test: someone should be able to ship "Traveller-like sci-fi" as a ruleset plus an asset pack, with no engine change. Keep that user story in a doc and check every rules-kernel PR against it.

### 12.3 Client-side asset replacement and fairness

A client replacing a goblin model with a bright pink box is fine in single-player and a competitive question in multiplayer. Policy: the server publishes an `asset_policy` (`open` / `hash-locked`), and hash-locked servers verify the digest of the client's `assets.lock` against the resolved `campaign.lock` or packaged manifest. Do not build an anti-cheat beyond this.

---

## 13. Performance targets and what to defer

### 13.1 Targets to design against (not to hit today)

Windows owns the primary product/performance baseline (ADR-0012). Linux
server performance remains a future supported-target concern; T009c does
not implement a new performance gate or define cross-target equality.

| Quantity | Target | Notes |
|---|---|---|
| Entities per area, active | 200 | plus ~2,000 static props |
| Entities per area, ceiling | 1,000 | AI budgeted, not all thinking each tick |
| Players per server | 8 initially; 32 designed-for | area-scoped worlds are the lever |
| Server tick | 20 Hz; ≤ 8 ms per area per tick at target load | measured, with a CI perf gate |
| Area size | 500 × 500 m | one navmesh, one nav bake |
| Bandwidth per client | ≤ 15 KB/s steady, ≤ 60 KB/s in busy combat | delta compression + component filtering |
| Pathfinding | ≤ 0.5 ms average per query, cached per (start-cell, goal-cell) | |
| Client frame | 60 fps at 1080p on a 2019 mid-range GPU | Godot forward+ handles this easily at these entity counts |
| Save file | ≤ 20 MB for a large campaign state; write ≤ 200 ms | |
| Memory, server | ≤ 500 MB per loaded area | |

### 13.2 Design for scale now (cheap)

- One `World` per area, no shared mutable state between areas.
- `InterestSet` as an interface even though the first implementation is "the whole area".
- Component storage dense and contiguous; no per-entity heap allocation in hot paths.
- No O(n²) loops over entities anywhere. Spatial queries go through `SpatialIndex` (uniform grid). Enforce by review.
- Budgeted AI scheduling from the very first AI implementation.
- Message-based cross-area communication.
- Measure from the start: a `crpgc bench` command that loads a synthetic 1,000-entity area and reports per-system tick times. Run it in CI and fail on a 20% regression.

### 13.3 Deliberately simple (do not optimise)

Single-threaded tick per area. No job system. No SIMD. No custom allocator. No streaming within an area. No LOD system beyond what Godot does automatically. No multi-server sharding. No delta compression beyond "send changed components". No client-side world caching between sessions.

Revisit only when the benchmark says so, with a profile attached to the issue.

---

## 14. Repository structure

```
crpg/
├─ README.md
├─ AGENTS.md                  ← how agents work in this repo (root contract)
├─ Cargo.toml                 ← workspace
├─ rust-toolchain.toml        ← pinned toolchain; agents must not change it
├─ deny.toml                  ← licence + dependency policy
├─ docs/
│  ├─ architecture/           system-by-system design docs (this document, split)
│  ├─ adr/                    numbered, immutable architecture decision records
│  ├─ contracts/              cross-crate API contracts + invariants
│  └─ guides/                 authoring guides for campaign creators
├─ crates/
│  ├─ crpg-core/              ids, fixed-point, RNG, tick/time, generic event substrate, errors
│  ├─ crpg-data/              campaign schema, serde, validation, migrations
│  ├─ crpg-rules/             stats, modifiers, effects, resolution, resources
│  ├─ crpg-sim/               world store, systems, tick, spatial, movement, LOS
│  ├─ crpg-script/            Lua sandbox, event IR, graph compiler, continuations
│  ├─ crpg-ai/                utility scorer, behaviour trees, influence maps
│  ├─ crpg-nav/               Recast/Detour bake + query
│  ├─ crpg-net/               protocol, codec, quinn transport, interest, filtering
│  ├─ crpg-persist/           snapshot backend, save format, migration
│  ├─ crpg-edit/              campaign document, edit commands, undo, validation
│  ├─ crpg-contracts/         shared traits ONLY. Human-owned. Rarely changes.
│  ├─ crpg-testkit/           fixtures, harnesses, replay runner, state hashing
│  ├─ crpg-server/            dedicated binary; shared embedded host placement owned by E012/E022
│  ├─ crpg-cli/               [bin] crpgc: validate, fmt, migrate, pack, run, replay
│  └─ crpg-godot/             [cdylib] GDExtension bridge for client + editor
├─ apps/
│  ├─ client/                 Godot project: scenes, UI, GDScript views
│  └─ editor/                 Godot project: editor UI, panels, graph views
├─ rulesets/
│  ├─ minimal-d6/             abstraction test #1
│  ├─ srd-lite/               abstraction test #2
│  └─ pf2e/                   separately licensed. Own LICENSE and NOTICE.
├─ campaigns/
│  ├─ fixtures/               tiny campaigns used by automated tests
│  └─ greenhollow/            the vertical-slice demo campaign
├─ schemas/                   generated JSON Schemas (checked in, drift-tested)
├─ tools/                     build scripts, patch applier, asset importers
├─ third_party/
│  └─ godot/                  pinned tag (submodule) + patches/*.patch
└─ .github/workflows/         CI
```

Why each part exists, briefly: `crpg-contracts` is the human-owned choke point that keeps agents from redefining interfaces unilaterally. `crpg-testkit` exists so that test infrastructure is a dependency rather than copy-pasted into every crate. `schemas/` is checked in so external tools and agents can read it without building. `third_party/godot/` holds a pinned tag and a patch queue whose size is a tracked metric. `rulesets/pf2e/` is physically separate so its licence obligations never contaminate the engine.

---

## 15. AI-agent development infrastructure

This section is what makes the rest of the plan achievable by one person.

### 15.1 The core idea: crates are agent territory

Each crate is a work unit with an owner, a public API, an invariant list, and a test command. Two agents working in two crates cannot break each other except through `crpg-contracts`, which they are not permitted to change.

Every crate has an `AGENTS.md`:

```markdown
# crpg-rules — agent contract
## Purpose
Resolve stat values, apply modifiers, decide outcomes. Ruleset-agnostic.
## Public API  (changing this requires an ADR)
StatQuery, ModifierPipeline, ResolutionRequest/Outcome, EffectStore, ResourcePool
## Invariants
- No floating point. Integers or Fx16_16 only.
- No HashMap iteration. IndexMap or BTreeMap only.
- No I/O, no clock, no threads, no randomness except through `crpg_core::Rng`.
- Never mention a specific game system (no "AC", "d20", "Strength").
## Allowed dependencies
crpg-core, crpg-data, indexmap, serde, thiserror.  Adding a dependency needs approval.
## Definition of done for any change
cargo test -p crpg-rules && cargo clippy -p crpg-rules -- -D warnings
&& cargo test -p crpg-testkit --test rules_golden
## Known traps
- Modifier stacking is ruleset policy, not kernel policy. Do not hardcode.
- Every query must produce a ModifierBreakdown, even in hot paths.
```

The root `AGENTS.md` states the non-negotiables: dependency direction, no `unsafe` outside `crpg-godot`, no new dependencies without approval, no schema version bump without a migration and golden fixture, no changes to `crpg-contracts` or `rust-toolchain.toml`, and "if the task requires violating a contract, stop and write an ADR proposal instead".

### 15.2 Subsystem boundaries that are actually safe for parallel work

Your suggested split is close, but the safe boundaries follow the *data* rather than the job titles. Ranked by isolation:

**Highly parallel-safe (agents can work simultaneously with near-zero conflict):**
- `crpg-nav`, `crpg-ai`, `crpg-persist`, `crpg-script` (Lua sandbox), the CLI, individual editor panels, ruleset data authoring, test fixtures, documentation.

**Parallel-safe with a contract:**
- `crpg-net` (protocol changes need a version bump and a contract test), `crpg-rules` (kernel API is a contract), `crpg-edit` (command enum is a contract).

**Serialise these — one agent at a time, human review:**
- `crpg-sim` tick order and the `World` struct. Changing tick order changes behaviour globally.
- `crpg-contracts`.
- Schema versions and migrations.
- The GDExtension boundary.
- Anything that re-blesses golden test outputs.

Practical rule: **an agent task should touch one crate plus its tests.** A task that requires touching three crates is a design problem that a human should resolve into an ADR plus three single-crate tasks.

### 15.3 Contract tests

For every trait in `crpg-contracts`, `crpg-testkit` provides a **conformance test suite** that any implementation must pass:

```rust
// crpg-testkit
pub fn assert_persistence_backend<B: PersistenceBackend>(make: impl Fn() -> B) { /* ~30 tests */ }
pub fn assert_transport<T: Transport>(make: impl Fn() -> (T, T)) { /* ordering, loss, reconnect */ }
pub fn assert_ruleset<R: Ruleset>(r: &R) { /* stat declarations resolve, tables total, etc. */ }
```

An agent implementing a new backend calls one function and knows immediately whether it is correct. This is the highest-value piece of test infrastructure in the project.

### 15.4 The integration gate

Every change, human or agent, passes the same pipeline. This is the "integration agent" in your brief, and it should be CI plus a merge queue rather than an LLM, because determinism beats judgement here.

```
1.  cargo fmt --check
2.  cargo clippy --workspace --all-targets -- -D warnings
3.  cargo deny check              (licences, advisories, banned crates)
4.  dependency-direction lint     (custom: crpg-rules must not import crpg-sim, etc.)
5.  determinism lint              (custom: no HashMap iteration or f32/f64 in
                                   crpg-rules / crpg-sim rules paths; no SystemTime)
6.  cargo test --workspace
7.  schema drift check            (regenerate schemas, diff against schemas/)
8.  validate every fixture campaign + rulesets/*  (crpgc validate)
9.  golden replay tests           (independent Windows/MSVC and Linux/GNU scoped goldens)
10. save/load equivalence test
11. perf gate                     (crpgc bench vs stored baseline, 20% tolerance)
12. build Windows client + editor + server and Linux headless server artifacts
13. smoke tests: Windows embedded + dedicated server, Linux dedicated server,
    each with a scripted client completing the fixture campaign
```

Steps 4, 5, 9, and 13 are the ones that specifically catch agent mistakes that a compiler will not. Write them early; they pay for themselves within weeks.

Steps 7–13 are **capability-gated**, not placeholder jobs (E020, 2026-09-06).
A gate becomes mandatory in the task that first supplies something real for
it to check: schema drift with T10; fixture validation with T11's data/CLI
split; replay with T9a corrected by T009c/ADR-0012; save/load equivalence with
persistence; performance with E019's benchmark task; and product builds and
smokes as their capabilities exist. Future product acceptance requires real
Windows client/editor/server and Linux headless server artifacts plus Windows
embedded-server and dedicated-server smokes and a Linux dedicated-server
smoke. Linux headless gates must not depend on Godot. An unavailable gate is
roadmap work, not a skipped green CI job; T009c adds no placeholder jobs.

T009c must make the existing Windows and Linux `cargo test --workspace` jobs
each perform a real `play_and_verify` comparison against that target's own
independently generated scoped golden (ADR-0012, §16.1). Selection is at
compile time, not by runtime OS. A missing baseline fails with `Io(NotFound)`;
no tolerance or skipped-green fallback is allowed. Other targets/profiles
keep portable replay shape/repeatability coverage without reading either
scoped golden. Step 9 does not wait for unrelated steps 7, 8, or 10–13.
All required T009c gates passed on native Windows/MSVC and genuine Linux/GNU
in WSL Ubuntu 24.04; see the [completion record](../tasks/T009c.md).
T009c implementation/verification and final audit are complete in the working
tree awaiting review/merge. T009c remains the priority before T009b; T009a is
also uncommitted.

A merge queue (GitHub merge queue or a simple `bors`-style bot) rebases each branch onto main and runs the full pipeline before merging. Agents working in parallel then cannot land a combination that individually passed and jointly fails.

### 15.5 Task specification format for agents

Every issue an agent picks up uses this template. Vagueness is the main cause of bad agent output, and this template removes most of it:

```markdown
## Task
One sentence.
## Crate(s)
crpg-ai  (only)
## Purpose
Why this exists, in two sentences, referencing an ADR if applicable.
## Interface
Exact signatures to add or change. If none, say "no public API change".
## Constraints
- Must not add dependencies.
- Must not allocate per-entity per-tick.
- Deterministic: same world + seed → same decision.
## Test
Exact test to write, and the exact command that must pass.
## Definition of done
Checklist. Includes "AGENTS.md updated if the public API changed".
## Out of scope
Explicit list. Prevents helpful expansion.
```

The `Out of scope` field is not decoration. It is the single most effective anti-scope-creep device available when working with agents.

### 15.6 Documentation discipline

- **ADRs** are numbered, dated, immutable, and short (one page: context, decision, consequences, alternatives rejected). Superseding an ADR means writing a new one, not editing the old one. Agents cite ADR numbers in commit messages.
- `docs/architecture/` mirrors the crate list one-to-one. If a crate has no architecture doc, it is not ready for agent work.
- Every public item has a doc comment. `#![warn(missing_docs)]` on every library crate.

---

## 16. Testing strategy

Testing is the load-bearing element of an agent-built codebase. Budget roughly 30–40% of effort here and do not treat it as overhead.

### 16.1 The foundation: deterministic headless simulation

Everything else depends on this, so build it in Phase 1, not Phase 6.

```
crpg-server --deterministic --campaign campaigns/fixtures/combat_basic \
            --seed 12345 --input tests/replays/combat_basic.replay \
            --ticks 2000 --hash-every 100 --out-hashes result.txt
```

`state_hash(world)` is a stable BLAKE3 hash over the canonical serialization
of the world, subject to the governed exclusions below. Golden files store
expected hash sequences. A change in behaviour shows as a hash divergence
at a specific tick. Replay reporting carries replay identity and a structured
mismatch with only the present sides (ADR-0010); it does not provide a
semantic world-state diff. Semantic diff tooling is deferred.

Exclusions are governed by ADR-0009, not discretionary: the list starts
empty, tick and queue bytes are included, and a proposed exclusion requires
a test proving it cannot affect behaviour. Presentation-only caches require
test plus review; admitting non-deterministic `World` state or changing that
governance requires an ADR. ADR-0012 changes only Decision 3's Linux-only
comparison selection, not this rule or the exact-build scope.

**Replay baseline contract (ADR-0012):** Windows/MSVC owns the primary
behavioural baseline; Linux/GNU owns a fully supported server regression
baseline. Neither target compares its hashes to the other. Under pinned
Rust 1.98.0, normal test profile, and default features, the portable
`crates/crpg-testkit/fixtures/replay_basic.replay` must be compared against:

```
crates/crpg-testkit/goldens/replay_basic_rust-1.98.0_x86_64-pc-windows-msvc_test-default.golden
crates/crpg-testkit/goldens/replay_basic_rust-1.98.0_x86_64-unknown-linux-gnu_test-default.golden
```

The real comparison branches are selected with
`cfg(all(target_os = "windows", target_arch = "x86_64", target_env = "msvc", debug_assertions))`
and
`cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu", debug_assertions))`.
No runtime OS selection, tolerance, cross-target equality assertion, or
missing-baseline skip is allowed. Unsupported targets/profiles retain
portable parse/play repeatability and shape tests without scoped goldens.
The toolchain, full target triple, profile, and feature set are filename
scope; changing any element requires reviewed re-baselining.

Generate independently in genuine native environments through production
`read_replay` -> `play_replay` -> `write_golden`, never by copying Linux hashes
to Windows even if they match. T009a's genuine Ubuntu 24.04/Rust 1.98.0 Linux
provenance is retained; renaming its matching baseline is allowed but is not
regeneration, and post-change Linux verification is still required. Record
commands, `rustc -vV` target, results, test counts, and golden SHA-256 for
both environments, including missing-baseline failure proof. Replay/golden
artifacts must have canonical LF endings in checkout/index/worktree and no
BOM. T009c's required native gates passed on Windows/MSVC and genuine
Linux/GNU in WSL Ubuntu 24.04; the [completion record](../tasks/T009c.md)
records verification and provenance. This is working-tree verification,
not review/merge approval; final audit and implementation/verification are
complete in the working tree awaiting review/merge.

This one facility gives you: regression detection, bisectable behaviour changes, save/load verification, multiplayer desync detection, and a scientific answer to "did my refactor change anything?"

### 16.2 By layer

**`crpg-core`** — property tests: RNG reproducibility and stream independence, fixed-point arithmetic identities, id generation uniqueness, event-queue ordering over the generic envelope (ADR-0008; game payloads covered in sim).

**`crpg-data`** — round-trip property tests (`parse(write(x)) == x`) via `proptest`; every fixture validates; every migration has a before/after golden; canonical formatter idempotence; malicious input tests (deep nesting, huge numbers, duplicate keys, zip bombs, path traversal).

**`crpg-rules`** — the largest unit test suite in the project. Table-driven cases per ruleset: `given stats + modifiers + roll, expect outcome and breakdown`. Property tests: modifier application is order-independent within a stacking group; removing an effect exactly reverses its modifiers; no query panics on missing stats. Explicit edge cases: zero and negative values, overflow, conflicting `Set` modifiers, effects whose source no longer exists, circular derived stats (must be detected at load, not at query).

**`crpg-sim`** — deterministic scenario tests: a fixture world, a scripted input sequence, assertions on the resulting state. Save/load equivalence after every scenario. Tick-budget tests. Spatial index correctness against brute force (property test).

**`crpg-testkit`**: portable replay behaviour plus the two compile-time
Windows/MSVC and Linux/GNU exact comparisons in §16.1. Each supported target
must fail on its own regression or missing baseline, not merely pass a shape
check. Cross-layer tests remain in this integration consumer, not dependencies
of the core/data/rules/sim layers.

**Authoritative hosting**: once implemented, exercise the same host through
Windows embedded and dedicated adapters and the Linux dedicated adapter,
including scripted-client fixture completion and the unchanged authority
boundary. Linux headless tooling and server-side extension tests are fully
supported, Godot-free gates, not best-effort coverage (ADR-0012).

**`crpg-script`** — sandbox escape tests (attempt `io`, `os.execute`, `require`, `debug`, FFI; all must fail); budget enforcement tests (infinite loop must abort, not hang); continuation serialization tests (a graph paused in `Wait` survives save/load); determinism tests (`pairs` ordering, `math.random`).

**`crpg-net`** — codec round-trip property tests; a simulated-network transport with configurable latency/jitter/loss/reorder; **malicious client tests as a first-class suite**: oversized packets, malformed frames, intents for entities not owned, intents at illegal times, flooding, replay of old packets, requesting entities outside the interest set. Every one of these must produce a clean rejection and a log line, never a panic. Desync test: run server and client replicas over the simulated network for 5,000 ticks and assert the replica's visible state matches the server's filtered projection.

**`crpg-edit`** — command/undo property test: apply a random valid command sequence, undo all of it, assert the document equals its initial state byte-for-byte after canonical serialization. This single test catches most editor corruption bugs. Also: validation catches every seeded defect in a deliberately-broken fixture campaign; reference integrity after deletes and renames.

**Campaign level** — for each fixture campaign, a scripted playthrough that must complete: enter area, talk, accept quest, fight, loot, complete quest, save, load, verify state. Run in CI on every commit. When this test breaks, the product is broken.

**Client/rendering** — on primary Windows/MSVC, the weakest area, and that is acceptable. Do smoke tests (client launches, connects, loads an area, renders 60 frames without error) and a handful of screenshot comparisons with generous tolerance for UI layout, never for replay hashes. Do not build a rendering test framework; Linux GUI support is not promised (ADR-0012).

**Editor** — headless tests of the document API cover the important logic on both supported targets. For Windows UI, a scripted-input smoke test that opens each editor type and saves without error. Nothing more; Linux headless tooling must not require Godot (ADR-0012).

### 16.3 Fixtures

Keep a small set of deliberately tiny campaigns, each testing one thing, versioned in `campaigns/fixtures/`:

`empty` · `one_area_one_creature` · `combat_basic` · `dialogue_branching` · `quest_multistage` · `triggers_and_doors` · `save_load_stress` · `broken_references` (must fail validation with exactly N diagnostics) · `migration_v1` (old-schema campaign that must still load).

Fixtures must stay tiny. A fixture that takes 30 seconds to run will be skipped, and a skipped test is worse than no test because it lies.

---

## 17. The MVP

### 17.1 Scope, stated as a boundary

The MVP is **one area, two creatures, one NPC, one quest, one weapon, one ruleset with about twelve stats**, running through the real client/server architecture. Its purpose is to prove the pipeline `Editor → Campaign Data → Server → Client`, not to be fun.

### 17.2 In scope

1. Editor creates an area with a flat or simple heightfield terrain and a few placed props.
2. Place a player start, one NPC, one hostile creature.
3. Creatures use `minimal-d6`: 3 attributes, HP, one attack, 2d6 resolution.
4. One dialogue with three nodes and one choice that starts a quest.
5. One quest with two states: started, completed. Completion triggered by the enemy's death.
6. Press Play: editor spawns an in-process server, connects a client.
7. Click-to-move over a baked navmesh, with server authority and client prediction.
8. Talk to the NPC, choose the option, quest starts and appears in the journal.
9. Attack the enemy, turn-based, real dice rolls, combat log, enemy dies.
10. Quest completes, journal updates.
11. Save. Quit. Load. State is identical.
12. Run the same campaign against a **separate** `crpg-server` process with **two** clients connected, and verify both see the same world.

### 17.3 Explicitly out of scope for the MVP

No spells. No inventory UI beyond equipping one weapon. No character creation. No levelling. No PF2e. No shops. No traps. No cinematics. No music. No lighting beyond a directional light. No visual event graph editor (the MVP's trigger is one line of Lua or a hardcoded quest hook). No animation blending beyond idle/walk/attack/die. No AI beyond "move to nearest hostile and attack". No modding UI. No packaging UI.

The temptation you must resist by name: **implementing PF2e's action economy because it is more interesting than 2d6.** If the MVP ships with PF2e, the abstraction will be wrong and you will not find out for a year.

### 17.4 The first playable prototype: "Greenhollow"

Immediately after the MVP, one step up: an area with real terrain and buildings, three NPCs, a small goblin camp with four enemies, a two-stage quest, a locked door with a key, a chest with loot, a shop with three items, and the `srd-lite` ruleset with levels 1–2. Two players can play it together. This is the artifact you show people, and it is the first honest test of whether the editor is usable.

---

## 18. Roadmap

I have reordered your phases in four places, for reasons given below.

### Phase 0 — Feasibility spikes (2–3 weeks)

**Objective:** kill the project cheaply if it should be killed.
**Deliverables:** (a) a `godot-rust` spike rendering 200 skinned characters driven entirely by an external Rust state array at 60 fps, with no gameplay logic in Godot; (b) a `quinn` spike moving a capsule with prediction and reconciliation between two processes; (c) a `mlua` sandbox spike proving budget enforcement and escape resistance; (d) ADRs 0001–0010 recording the decisions in Section 22.
**Definition of done:** all three spikes run; each has a written go/no-go with measured numbers.
**Risks:** GDExtension performance for many skinned meshes; `godot-rust` API friction.
**Do not build:** anything permanent. These are throwaway.

### Phase 1 — Core skeleton and the test harness (4–6 weeks)

**Objective:** the workspace, the world store, the tick loop, and the deterministic replay harness.
**Deliverables:** `crpg-core`, `crpg-sim` (entities, components, a trivial movement system), `crpg-testkit` with state hashing and the replay runner, `crpg-cli` with `run`/`replay`, CI pipeline steps 1–6 and 9.
**Dependencies:** Phase 0.
**Tests:** replay determinism over 10,000 ticks; save/load equivalence.
**Definition of done:** `crpgc replay fixture.replay` reproduces the exact-build hash sequence, and CI enforces independent Windows/MSVC and Linux/GNU target-scoped goldens under the pinned toolchain, normal test profile, and default features (ADR-0012). Two machines running the same binary must agree; different targets are not compared. T009c precedes T009b despite its suffix.
**Do not build:** networking, rendering, rules, or an editor.

> **Reordering note:** the deterministic harness moves from Phase 6 to Phase 1. It is infrastructure, not a feature, and everything after this is cheaper because it exists.

### Phase 2 — Campaign data format (3–5 weeks)

**Objective:** the format, the schemas, the loader, validation, migrations, packaging.
**Deliverables:** `crpg-data`, generated `schemas/`, `crpgc validate|fmt|new|pack|schema|explain`, fixture campaigns, migration framework with one dummy migration and its golden test.
**Tests:** round-trip properties, validation diagnostics on `broken_references`, migration goldens, malicious archive tests.
**Definition of done:** an LLM given only `crpgc schema creature` output produces a valid creature file on the first try. Test this literally.
**Do not build:** the editor GUI, assets, or the visual graph format.

### Phase 3 — Rules kernel and two throwaway rulesets (5–7 weeks)

**Objective:** prove the abstraction before it has customers.
**Deliverables:** `crpg-rules` (stats, modifier pipeline with breakdowns, resolution, outcome tables, resources, effects, hooks), `rulesets/minimal-d6`, `rulesets/srd-lite`.
**Tests:** the large table-driven rules suite; both rulesets pass `assert_ruleset`; no kernel change was required to add the second ruleset.
**Definition of done:** a combat encounter resolves headlessly under both rulesets from the same fixture campaign.
**Risks:** the abstraction leaks. Mitigation is precisely the second ruleset.
**Do not build:** PF2e. Not one spell.

> **Reordering note:** the rules kernel moves ahead of the client, the server, and the editor. It is the component most likely to be wrong, it is cheap to test headlessly, and discovering its flaws after the editor depends on it is expensive.

### Phase 4 — Server and networking (6–8 weeks)

**Objective:** authoritative simulation over a real network.
**Deliverables:** `crpg-net` (protocol, codec, quinn, sessions, interest sets, per-client filtering), `crpg-server` binary, simulated-network test transport, malicious-client suite.
**Tests:** desync test over 5,000 ticks with loss and jitter; every malicious-client case; reconnection.
**Definition of done:** two headless scripted clients connect, move, fight, and end with identical visible state.
**Platform acceptance (ADR-0012):** one reusable authoritative host serves
Windows embedded single-player, Windows dedicated, and Linux dedicated
adapters, with real smoke tests as each exists. E012/E022 own the API/package
decision; headless Linux builds must not depend on Godot.
**Do not build:** prediction polish, AOI within areas, a master server.

### Phase 5 — Client (6–8 weeks)

**Objective:** see and control the simulation.
**Deliverables:** `crpg-godot` bridge, scene sync, camera, click-to-move with prediction, basic HUD, combat log, dialogue UI, `SimEvent`-driven VFX and audio hooks.
**Definition of done:** a human plays the `combat_basic` fixture end to end at 60 fps.
**Do not build:** inventory art, character sheets, spell UI, cinematics, settings menus.

### Phase 6 — Editor v1 (8–12 weeks)

**Objective:** create the MVP campaign without hand-writing JSON.
**Deliverables:** `crpg-edit` document/command/undo/validate; editor shell, campaign explorer, generated property forms, area editor with terrain and placement, creature editor, dialogue editor with text round-trip, quest editor, problems panel, Play.
**Definition of done:** the MVP campaign (Section 17.2) is created entirely in the editor by someone who has never seen the JSON.
**Risks:** this phase is where scope explodes. Mitigation: the property forms are generated, not written, and no editor ships without a corresponding headless command API.
**Do not build:** the visual event graph editor, cinematics, the rules editor, packaging UI.

### Phase 7 — MVP integration and hardening (3–4 weeks)

Ship the MVP. Fix what the integration reveals. Write the authoring guide. This phase exists because integration always reveals something and pretending otherwise corrupts every subsequent estimate.

### Phase 8 — Event graphs, dialogue depth, quests (5–7 weeks)

Visual graph editor over the IR, graph compiler, continuations that survive save/load, quest reachability analysis, triggers, doors, containers, traps, shops, loot tables.

### Phase 9 — AI (4–6 weeks)

Utility scoring, AI profiles as data, behaviour trees for schedules, party AI, the encounter dry-run simulator.

### Phase 10 — Multiplayer hardening (4–6 weeks)

Multi-client editor testing with a network simulator, reconnection, party management, host migration decision (recommend: no multiplayer host migration, not removal of Windows embedded single-player), admin tooling, and public Windows/MSVC and Linux/GNU dedicated-server builds (ADR-0012).

### Phase 11 — PF2e (12–20 weeks, and it will be more)

Levels 1–5. Core action economy. About 40 classes-worth of features cut to 4 classes. Roughly 150 spells. Conditions. This phase is enormous and should be treated as a content project with its own sub-roadmap, largely data-authored, heavily agent-assisted, and continuously validated by the rules test suite. Legal review of ORC compliance happens at the start of this phase, not at the end.

### Phase 12 — Modding, packaging, distribution (4–6 weeks)

Ruleset packaging, campaign publishing, dependency resolution, signing, the third-party server extension story (WASM if needed), and documentation.

Windows is the primary product/release target; Linux dedicated server,
headless tooling, and server-side extensibility remain fully supported.
Portable T1 data/sandboxed Lua and target-specific T0 native artifacts must
remain distinct. Resolve §12.1's open native-loading/packaging decision before
implementing it, without assuming a stable ABI or an unsafe exception
(ADR-0012).

### Phase 13 — Polish, performance, platforms

Ongoing thereafter.

---

## 19. Milestone backlog at three scales

### 19.1 Very small (a few hours to one day)

These are real backlog items, ordered roughly as they become available.

1. Cargo workspace with all crates as empty stubs, CI running `fmt` + `clippy`.
2. `EntityId` with generational indices, plus a property test for id reuse safety.
3. `Fx16_16` fixed-point type with arithmetic and property tests.
4. `DeterministicRng` (PCG or xoshiro) with named sub-streams and a reproducibility test.
5. The dependency-direction lint as a CI script.
6. The determinism lint (ban `HashMap` iteration and `f64` in listed crates).
7. `ComponentStore<T>` with dense storage, insert/remove/iterate, and tests.
8. `state_hash(world)` over canonical serialization.
9. `Tick`, `Duration`, and the fixed-step loop with an accumulator.
10. `crpgc new <type>` scaffolding one document type.
11. Canonical JSON writer plus an idempotence test.
12. ULID generation and parsing.
13. `StatId` interning and the `StatBlock` type.
14. One `Modifier` applied to one stat, with a `ModifierBreakdown`.
15. `DiceExpr` parser (`2d6+3`, `4d6kh3`) with property tests.
16. `OutcomeTable` evaluation with the PF2e-style four-band table as test data only.
17. `ResourcePool` with refresh triggers.
18. The Lua sandbox environment (deny list) with escape tests.
19. Codec round-trip test for one message type.
20. Simulated-network transport with configurable loss.
21. Recast navmesh bake for one hardcoded box-shaped area.
22. A Godot scene that spawns N `Node3D` proxies from an external array.
23. The client's interpolation buffer with a unit test.
24. The editor's campaign explorer tree, read-only.
25. Schema-generated property form for one document type.
26. The problems panel rendering `Vec<Diagnostic>`.
27. One ADR, written properly, as a template for the rest.

### 19.2 Medium (several days to about two weeks)

1. The world store with serialization and a full save/load equivalence test.
2. The replay harness with recording, playback, hash comparison, and a divergence report.
3. Campaign loader with full validation and positioned diagnostics.
4. The migration framework with two versions and golden fixtures.
5. The modifier pipeline with stacking policies and a 100-case test table.
6. The resolution pipeline covering checks, saves, and attacks under two rulesets.
7. The effect store with durations, lifecycle hooks, and removal correctness.
8. `minimal-d6` complete, driving a headless combat.
9. Protocol v1 with handshake, area load, deltas, and intents.
10. Interest sets plus per-client component filtering, with a leak test.
11. Movement prediction and reconciliation.
12. Scene sync on the client with proxy lifecycle.
13. Dialogue engine plus dialogue UI.
14. Quest state machine plus journal.
15. The document/command/undo system with the random-command-sequence property test.
16. The area editor's placement tooling (select, move, rotate, snap, duplicate, delete).
17. The terrain heightfield editor with sculpt and paint.
18. Navmesh baking integrated into the editor with a walkable overlay.
19. The event IR interpreter with serializable `Wait` continuations.
20. The utility AI scorer with data-driven weights.
21. The malicious-client test suite.
22. Packaging to `.crpg` with hashing and signing.
23. The encounter dry-run simulator.
24. The multi-client test launcher with network condition simulation.

### 19.3 Major (weeks to months)

1. Phase 1 core plus deterministic harness.
2. Phase 2 campaign format end to end.
3. Phase 3 rules kernel plus two rulesets.
4. Phase 4 authoritative server and networking.
5. Phase 5 playable client.
6. Phase 6 editor v1.
7. The MVP, integrated and documented.
8. Visual event graphs.
9. AI stages 1–3.
10. Multiplayer hardening and the first public dedicated server.
11. PF2e levels 1–5.
12. Modding and distribution.

---

## 20. Technical decisions

| # | Decision | Recommended | Alternatives | Reason | Reversibility | Risk |
|---|---|---|---|---|---|---|
| 1 | Engine relationship | **Pinned Godot 4.x + patch queue, consumed via GDExtension; core is Godot-free** | Deep fork; plugin; clean-room | Keeps upstream renderer improvements; server needs no engine; editor UX is unconstrained | **High** — the core has no Godot dependency | Medium: GDExtension API gaps |
| 2 | Core language | **Rust** | C++, C#, Zig | Compiler as reviewer for agent-written code; crates map to agent boundaries; serde + schemars; no GC on the server | Very low | Medium: learning curve, gdext maturity |
| 3 | Presentation glue | **GDScript (typed) for views only** | C#, Rust for all UI | Fast iteration on UI; keeps FFI surface small | Medium | Low |
| 4 | Build system | **Cargo workspace + `xtask` + SCons only for Godot patches** | CMake, Bazel, Meson | Cargo is the whole toolchain for the code that matters | Low | Low |
| 5 | Godot base | **Latest stable 4.x at Phase 0, pinned by tag; upgrade deliberately once per minor release** | Track master; freeze forever | Pinning keeps builds reproducible; deliberate upgrades keep the patch queue honest | High | Medium: upgrade churn |
| 6 | Renderer | **Godot Forward+ (Vulkan), unmodified** | Custom wgpu; Bevy; Mobile renderer | Best quality-per-effort available; not on the critical path | High (core is renderer-agnostic) | Low |
| 7 | Simulation substrate | **Purpose-built entity/component store, explicit systems, fixed order** | bevy_ecs; hecs; Godot SceneTree; OOP hierarchy | Determinism, serializability, agent comprehension; ECS perf is irrelevant at these counts | Medium | Low, but must resist framework creep |
| 8 | Determinism scope | **Exact-build replay; independent Windows/MSVC primary and Linux/GNU server regression goldens (ADR-0009, ADR-0012). No cross-platform lockstep.** | Full bit-determinism; Linux-only gate | Both authoritative hosts need real regression gates, not cross-target hash equality | High | Low |
| 9 | Physics | **Rapier/Parry server-side for queries; no client physics authority** | Godot physics; custom | Server has no Godot; queries (LOS, capsule sweeps, overlaps) are all a CRPG needs | Medium | Low |
| 10 | Navigation | **Recast/Detour in Rust; navmesh baked as a build artifact** | Godot NavigationServer; custom grid | Must run server-side; server and client must share the identical mesh | Medium | Medium: binding maturity |
| 11 | Networking transport | **QUIC via `quinn`** | ENet; raw UDP; TCP; WebRTC; Godot HLAPI | Encryption, reliable streams and datagrams, flow control, migration, all in one | Medium | Medium: QUIC through consumer NAT |
| 12 | Replication model | **Server-authoritative deltas + sim event stream; predict only own movement** | Lockstep; rollback; client authority | Correct for the genre; removes an entire bug class | Low | Low |
| 13 | Wire serialization | **`postcard` (compact binary) with an explicit protocol version** | bincode; protobuf; flatbuffers; JSON | Small, fast, serde-native, no schema compiler | High | Low |
| 14 | Campaign format | **Canonical JSON, entity or explicit aggregate document per file, object ULIDs plus package ids, generated JSON Schema** | RON; YAML; TOML; SQLite; custom DSL | LLM fluency, git diffs, tooling, schema validation | Medium (a converter is writable) | Medium: verbosity |
| 15 | Save format | **`postcard` + `zstd` world snapshot, atomic write** | JSON; SQLite; custom | Fast, small, exact; snapshot semantics are simplest to verify | High (trait-backed) | Low |
| 16 | Persistent-world DB | **Deferred. `PersistenceBackend` trait now, SQLite later** | Build SQLite now; Postgres | Premature; the trait costs nothing | High | Low |
| 17 | Scripting | **Lua 5.4 via `mlua`, hard-sandboxed, server-only** | GDScript (impossible); Rhai; Wren; WASM; C# | Precedent, LLM fluency, budgetable, sandboxable | Medium | Medium: sandbox correctness |
| 18 | Untrusted server plugins | **Deferred; WASM (`wasmtime`) when needed** | Native only; Lua only | Right tool, wrong time | High | Low |
| 19 | Visual scripting | **Graph UI compiling to the same IR as scripts** | Separate graph runtime; Blueprint-style bytecode | One executor, one debugger, one test surface | Medium | Medium: IR expressiveness |
| 20 | Rules architecture | **Seven-primitive kernel; all systems as data + hooks** | Hardcode PF2e; universal RPG metamodel; per-system plugins in Rust | Only a small kernel survives contact with four rule systems | Low (this is the hardest thing to change) | **High — the top design risk** |
| 21 | AI architecture | **Utility scoring for combat; behaviour trees for schedules; influence maps later** | FSM only; GOAP; planners; ML | Best quality-per-complexity for CRPG combat | High | Low |
| 22 | Asset format | **glTF 2.0 in, Godot's import pipeline, content-hashed in `assets.lock`** | FBX; custom | Open, tool-supported, agent-inspectable | Medium | Low |
| 23 | Test framework | **`cargo test` + `proptest` + `insta` (snapshots) + custom replay harness** | Bespoke framework; nextest only | Built-in beats bespoke; the replay harness is the only custom piece | High | Low |
| 24 | CI | **GitHub Actions Windows/MSVC + Linux/GNU gates; Windows primary product runner; merge queue; capability-gated builds/smokes (ADR-0012)** | Linux-only baselines; placeholder jobs | Both targets need independent replay and real product coverage as implemented | High | Low |
| 25 | Modding boundary | **Target-specific T0 native artifacts (loader/ABI decision deferred) / portable T1 server-only sandboxed Lua and data / T2 client data-only (ADR-0012)** | Client scripting; unsandboxed | Joining a stranger's server must be safe; embedded hosting preserves authority | Low | Medium |
| 26 | Localisation | **String keys from day one; `locale/<lang>.json`** | Retrofit later | Retrofitting touches every editor form and every content file | Low | Low |
| 27 | Licence | **MIT or Apache-2.0 for the engine; ruleset packages licensed separately** | GPL; proprietary | Permissive maximises adoption and keeps Godot compatibility trivial | Low | Low |
| 28 | Platform and hosting | **Windows/MSVC primary development/product/release/baseline; Linux/GNU fully supported headless server/tooling/extensions/tests; one authoritative host (ADR-0012)** | Linux-only server; separate single-player sim | Windows embedded and Windows/Linux dedicated adapters preserve the same client/server authority boundary | Explicit support decision required | Target-specific failures are defects |

---

## 21. The twenty biggest risks

| # | Risk | Severity | Mitigation |
|---|---|---|---|
| 1 | **The rules abstraction is wrong** and PF2e forces a kernel redesign after a year of dependent work | Critical | Build `minimal-d6` and `srd-lite` in Phase 3, before PF2e. If adding the second ruleset requires a kernel change, the kernel is wrong and you fix it while it is cheap. Write the "total conversion to sci-fi" user story and check every kernel PR against it. |
| 2 | **Scope explosion.** Every phase invites the next feature | Critical | Section 22 is mandatory reading before every phase. Every agent task has an `Out of scope` field. A written rule: nothing enters the roadmap without something leaving it. |
| 3 | **Editor complexity swallows the project.** Editors are 60–70% of this kind of product | Critical | Generate property forms from schemas. Every editor feature must have a headless command equivalent first. Ship the ugly editor. Resist per-type bespoke UI. |
| 4 | **Solo developer burnout / project abandonment** | Critical | Phase ordering ensures each phase yields something usable alone: a rules library, a campaign format, a headless server. Ship the MVP publicly at month 9 for external motivation. Treat the 4–6 year estimate as real and plan life around it. |
| 5 | **Agent-generated code quality degrades the codebase invisibly** | High | Clippy at `-D warnings`, `#![forbid(unsafe_code)]` outside the bridge, mandatory tests per task, golden replay tests, perf gate, and human review of every `crpg-sim` and `crpg-contracts` change. Never merge agent work that only "compiles and looks right". |
| 6 | **Agents conflict on shared code** | High | One crate per task. `crpg-contracts` is human-owned. Merge queue with full-pipeline rebase. Tasks that span crates get decomposed by a human first. |
| 7 | **Multiplayer complexity underestimated**, especially per-client filtering and reconnection | High | Build the simulated-network transport and the malicious-client suite in Phase 4, not Phase 10. Test multiplayer from the editor continuously, not at the end. |
| 8 | **GDExtension proves inadequate** for a needed rendering feature | High | Phase 0 spike measures it. The patch queue is the first escape hatch, a wgpu or Bevy client the second. The Godot-free core makes the second one survivable. |
| 9 | **Godot upgrade churn** breaks the bridge each minor release | Medium | Pin by tag. Upgrade once per minor release, deliberately, on a branch, with the smoke test as the gate. Track patch-queue size as a metric; if it grows, escalate to an ADR. |
| 10 | **Determinism silently breaks** and replay tests become worthless | High | The determinism lint in CI. Golden hashes on every fixture on every commit. Never allow a "temporarily disabled" determinism test. |
| 11 | **Save/load breaks on real campaigns** after months of accumulated state | High | Save/load equivalence in CI from Phase 1. Migration goldens for every schema version. Never ship a schema change without a migration. |
| 12 | **Lua sandbox escape**, especially once servers run untrusted campaigns | High | Deny-list plus allow-list, budgets, no FFI, escape tests in CI, and a documented statement that campaign scripts run only on the server. Consider a security review before the first public dedicated-server release. |
| 13 | **Campaign format churn** breaks user content and destroys trust | High | Schema versions from day one, migrations mandatory, and a public policy: campaigns authored in any released version load in all later versions. |
| 14 | **PF2e legal exposure** (trademarks, ORC compliance, art) | Medium-High | Ruleset as a separate package with its own LICENSE and NOTICE. No Paizo art or trademarks. Read ORC properly at the start of Phase 11 and get a lawyer's hour if the project gains an audience. |
| 15 | **Asset pipeline complexity**: rigging, retargeting, LODs, materials for hundreds of creatures | Medium-High | Fixed animation slot sets. One humanoid rig standard. Buy or use CC0 asset packs for the demo. Do not build a character customisation system. |
| 16 | **Performance surprises on the server** at a few hundred entities with real AI | Medium | Perf gate in CI from Phase 1. Budgeted AI from the first AI commit. Profile before optimising, and put the profile in the issue. |
| 17 | **`godot-rust` (gdext) abandonment or breaking changes** | Medium | Small FFI surface. Pin the version. A C++ GDExtension bridge is the fallback and would be a few weeks, not a rewrite. |
| 18 | **Nobody uses it.** A technically excellent engine with no campaigns and no creators | Medium-High | Ship the Greenhollow demo early and publicly. Prioritise the editor's authoring UX over engine elegance from Phase 6 onward. Talk to NWN and Solasta module authors before designing the dialogue and quest editors. |
| 19 | **Godot's licensing/credits obligations mishandled** in a rebranded product | Low-Medium | Generate the third-party licence screen at build time from `licenses/`; CI fails if a dependency has no licence entry. |
| 20 | **The "one more abstraction" trap** in the kernel: a rules DSL, a query language, an entity metamodel | Medium | Every abstraction must be justified by two concrete rulesets that need it. If only PF2e needs it, it belongs in the PF2e ruleset. Put this sentence in `AGENTS.md`. |

---

## 22. What NOT to build

Postponed indefinitely. Each of these is tempting, defensible in isolation, and fatal in aggregate for a solo developer.

**Rules and content**
1. Full PF2e. Not before Phase 11, and then only levels 1–5 with four classes.
2. Any second ruleset beyond the two throwaway test rulesets, before Phase 11.
3. A crafting system, an economy simulation, or dynamic pricing.
4. Character appearance customisation beyond swapping a handful of meshes.
5. Random/procedural dungeon or terrain generation.
6. Balance analysis tooling beyond the encounter dry-run simulator.

**Engine and simulation**
7. A general-purpose ECS framework of your own.
8. A job system, SIMD, or a custom allocator.
9. Cross-platform bit-deterministic simulation.
10. Rollback or lockstep netcode.
11. Destructible environments, cloth, hair, or ragdolls.
12. A custom renderer, before Godot has actually failed you with a measurement attached.
13. Streaming within an area, or seamless open worlds.

**Networking and infrastructure**
14. Sharding, clustering, or cross-server travel.
15. A master server, matchmaking, or a lobby browser.
16. NAT punch-through or relays. Require port forwarding at first.
17. Voice chat.
18. Anti-cheat beyond server authority plus data filtering.
19. Accounts, cloud saves, or a web portal.

**Editor**
20. A visual shader graph.
21. In-editor 3D modelling, sculpting beyond terrain heightfields, or UV editing.
22. A full script debugger with breakpoints and watches.
23. Real-time collaborative multi-user editing. Git is your collaboration story.
24. A cinematic timeline editor, before Phase 12.
25. A plugin API for the editor itself.
26. Theming, layout customisation, or a docking system beyond a fixed three-pane layout.

**Product and platform**
27. Mobile or console ports.
28. Steam Workshop, a mod marketplace, or any storefront integration.
29. Localisation into actual languages. String keys now; translation later.
30. Accessibility features beyond keyboard navigation and scalable UI text. (Design so they are addable; do not build them yet.)
31. A GM/dungeon-master live-play mode. It falls out of the privileged-client architecture later, almost free. Do not build it early.
32. VR, XR, or controller-first UI.
33. A custom scripting language. Ever.

When you are tempted by one of these, the test is: **does the MVP or the current phase's definition of done require it?** If not, it goes on this list with a date.

---

## 23. Final architectural recommendation

If I were building this project as a solo developer using AI agents, this is exactly how I would structure it.

**One sentence:** a Rust simulation core with no engine dependency, a headless authoritative server, and two Godot applications (client and editor) that are thin views over that core, with Godot consumed as a pinned upstream dependency rather than forked.

**Platform commitment (ADR-0012):** Windows/MSVC is primary for development,
product, release gates, and behavioural baselines, including client, editor,
embedded single-player server, dedicated server, and CLI. Linux/GNU is fully
supported for dedicated server, headless tooling, server-side extensibility,
and testing; failures are defects. Linux GUI builds are not promised and
Linux headless support is Godot-free. Each target reproduces its own
independently generated exact-build replay golden, never the other's hashes.

**The five decisions that define the project:**

1. **The simulation is Godot-free Rust.** This is the decision everything else hangs from. It makes the server clean, the tests deterministic, the agents productive, and the engine choice reversible.
2. **The server is the only authority: one platform-neutral implementation serves Windows embedded single-player and Windows/Linux dedicated processes.** In-memory transport preserves the boundary; clients never mutate authoritative state directly. OS-specific hosting logic stays above core/rules/sim.
3. **The rules kernel has seven primitives and no game-system knowledge.** It is proven by two throwaway rulesets before PF2e is touched.
4. **Campaign data is canonical JSON with ULID identity and generated schemas**, designed so an LLM with a schema and a CLI can author valid content without a GUI.
5. **The editor is a privileged client of a running server**, and every editor operation exists as a headless command first.

**The shape:**

```
                          ┌──────────────────────────────┐
                          │  campaign/  (canonical JSON) │
                          │  ruleset/   (data + Lua)     │
                          └───────────┬──────────────────┘
                              load / validate / migrate
                                      │
   ┌──────────────────────────────────▼───────────────────────────────────┐
   │  crpg-server  (Rust, headless, authoritative, deterministic)         │
   │  ┌────────┬────────┬─────────┬────────┬──────────┬────────────────┐  │
   │  │ rules  │  sim   │ script  │   ai   │ persist  │  net (QUIC)    │  │
   │  └────────┴────────┴─────────┴────────┴──────────┴────────────────┘  │
   └──────────────┬──────────────────────────────────┬────────────────────┘
       deltas +   │                                  │  privileged session
       sim events │                                  │  + document commands
   ┌──────────────▼────────────┐        ┌────────────▼─────────────────────┐
   │ crpg-client               │        │ crpg-editor                      │
   │ Godot 4 + gdext bridge    │        │ Godot 4 + gdext bridge           │
   │ replica world, no logic   │        │ crpg-edit document + undo        │
   │ render / input / UI       │        │ generated forms, problems panel  │
   └───────────────────────────┘        └──────────────────────────────────┘
                    ▲                                  │
                    └──────── in-process server ───────┘   (Play button)
```

**What this buys you that the alternatives do not:** a server you can run on a $5 VPS, a test suite that can prove behavioural equivalence across refactors, a campaign format an AI agent can author natively, an editor whose complexity is bounded by generated UI, and an engine dependency you can replace in a quarter rather than a decade.

**What it costs you:** roughly two to three months of extra up-front work before anything is visible on screen, and the discipline to keep Godot types out of the core when it would be five minutes faster to let them in. The second cost is the one that will actually threaten the project. Put the rule in `AGENTS.md`, enforce it with a CI lint on `crpg-godot` being the only crate allowed to depend on `godot`, and never grant an exception.

---

## 24. The first eighteen tasks

Start here, in this order. Tasks 1–3 are spikes and should be thrown away.

---

**T1. GDExtension rendering spike**
*Purpose:* prove that Godot can render CRPG-scale content driven entirely by external state, which is the load-bearing assumption of the whole architecture.
*Affected:* throwaway repo; `godot-rust` (gdext); a Godot 4.x project.
*Dependencies:* none.
*Work:* a Rust GDExtension holding an array of 200 `{position, rotation, anim_state}` structs updated at 20 Hz. Godot creates 200 skinned `Node3D` proxies, interpolates between updates, and drives `AnimationTree` from `anim_state`. No gameplay logic in GDScript.
*Test:* measure frame time at 200, 500, and 1,000 characters on your target GPU; measure the per-tick FFI cost.
*Done when:* 200 characters at ≥ 60 fps with FFI under 1 ms per tick, and you have written the go/no-go in an ADR. If it fails, escalate to a Bevy spike before continuing.

**T2. QUIC movement spike**
*Purpose:* validate `quinn` for authoritative movement with prediction, including through a home router.
*Affected:* throwaway repo.
*Dependencies:* none.
*Work:* two processes; client sends `MoveTo` intents, server integrates at 20 Hz and returns positions with an input acknowledgement; client predicts and reconciles. Add a latency/jitter/loss shim.
*Test:* smooth local movement at 150 ms RTT and 3% loss; reconciliation does not visibly rubber-band; a connection succeeds over the public internet through NAT.
*Done when:* measured and written up in an ADR, including the NAT result.

**T3. Lua sandbox spike**
*Purpose:* confirm `mlua` can be locked down and budgeted.
*Affected:* throwaway repo.
*Dependencies:* none.
*Work:* an `mlua` environment with `io`, `os`, `require`, `debug`, `package`, `load`, `loadstring` removed; an instruction-count hook; a memory limit; a deterministic `pairs` and `math.random`.
*Test:* ten scripted escape attempts all fail; an infinite loop aborts within the budget rather than hanging; two runs with the same seed produce identical output.
*Done when:* all pass and the sandbox module is small enough to copy into `crpg-script` later.

---

**T4. Workspace, toolchain, and CI skeleton**
*Purpose:* the substrate every subsequent task depends on.
*Affected:* `Cargo.toml`, `rust-toolchain.toml`, `deny.toml`, all crate stubs, `.github/workflows/ci.yml`, root `AGENTS.md`.
*Dependencies:* T1–T3 decisions.
*Work:* create every crate from Section 14 as a stub with `#![forbid(unsafe_code)]` (except `crpg-godot`) and `#![warn(missing_docs)]`. CI runs `fmt`, `clippy -D warnings`, `test`, `cargo deny`.
*Test:* CI green on an empty workspace.
*Done when:* a deliberately-broken commit fails CI for the right reason.

**T5. Dependency-direction and determinism lints**
*Purpose:* make the two architectural invariants machine-checked before anyone can violate them.
*Affected:* `tools/lint/`, CI.
*Dependencies:* T4.
*Work:* a script parsing `Cargo.toml` files against an allowed-edges table, failing on any cycle or disallowed edge, and specifically failing if any crate other than `crpg-godot` depends on `godot`. A second script greps `crpg-rules`/`crpg-sim` for `HashMap` iteration, `f32`/`f64` in rules paths, `SystemTime`, and `std::thread`.
*Test:* a fixture commit that adds `crpg-rules → crpg-sim` fails; one that adds `godot` to `crpg-sim` fails.
*Done when:* both lints run in CI and their failure messages name the offending file and rule.

**T6. `crpg-core`: ids, fixed-point, RNG, time**
*Purpose:* the deterministic primitives everything else builds on.
*Affected:* `crates/crpg-core`.
*Dependencies:* T4.
*Work:* `EntityId` (generational), `Ulid`, `Fx16_16`, `DeterministicRng` with named sub-streams, `Tick`/`RoundCount`, an interning table for `StatId`/`TagId`, and the error types.
*Test:* property tests for id reuse safety, fixed-point arithmetic identities, RNG reproducibility, and sub-stream independence.
*Done when:* `cargo test -p crpg-core` passes and `crpg-core/AGENTS.md` documents the public API.

**T7. `crpg-sim`: world store and component storage**
*Purpose:* the data structure the entire simulation lives in.
*Affected:* `crates/crpg-sim`.
*Dependencies:* T6.
*Work:* `World` with a generational entity arena, `ComponentStore<T>` (dense, `IndexMap`-backed), spawn/despawn/query, the `Timeline` container (`BTreeMap<(InitiativeKey, EntityId)>`), the generic event substrate (`EventEnvelope<P>`/`EventQueue<P>`) plus `World.events: EventQueue<SimEvent>` (scoped core exception per ADR-0008), and full `Serialize`/`Deserialize` of the skeleton (no `StatBlock` — its string conversion pair is T014's per ADR-0006 D4).
*Test:* skeleton property test that `deserialize(serialize(w)) == w` after random spawn/despawn/mutate sequences; a test that despawn does not leave dangling component entries.
*Done when:* round-trip property test passes over 10,000 generated cases.

**T8a. `state_hash` and the fixed-step tick loop (`crpg-sim`)**
*Purpose:* the measurement instrument for every behavioural test in the project, plus the loop it measures.
*Affected:* `crates/crpg-sim`.
*Dependencies:* T7.
*Work:* `state_hash(&World) -> [u8; 32]` over canonical serialization with an explicit exclusion list (queue bytes included per ADR-0008; hash function — expected `blake3` — decided in the task file, which is also where it enters the dependency set); `fn tick(&mut World)` with a hand-written ordered system list (initially one trivial system); `Timeline` advance policy — every-tick real-time advance and `EndTurn` turn-based advance, reconciling §6.2 turn-start AI with the §10 per-tick loop (E015).
*Test:* two runs of 10,000 ticks from the same seed produce identical hash sequences; changing the seed changes them. Tests are self-contained `proptest` suites in `crpg-sim` (E005: `crpg-sim` does not dev-depend on `crpg-testkit`).
*Done when:* the hash-sequence tests pass over 10,000 ticks in CI.

**T8b. Hash-sequence harness (`crpg-testkit`)**
*Purpose:* the reusable golden-hash machinery every later behavioural task (T009 replay, T016 combat, …) builds on. First real code in `crpg-testkit`, so it also writes the crate's `AGENTS.md` and architecture doc.
*Affected:* `crates/crpg-testkit`.
*Dependencies:* T8a.
*Work:* `run_hash_sequence(seed, ticks)` over `crpg-sim` (a normal dependency, which the ALLOWED table already permits), golden-file write/compare helpers, and the convention later tasks use. The thin `crpgc run --ticks/--hash-every` CLI wrapper is `crpg-cli` work and lives in T013 (transferred per E004, 2026-09-06).
*Test:* a golden hash file produced and verified end-to-end through the harness API.
*Done when:* a scripted 10,000-tick run produces and verifies its golden file with no sim-side changes.

(T8a/T8b are spec §24's single T8, split per E004 Option A: one task, one crate.)

**T9a. Replay record/playback harness (`crpg-testkit`)**
*Purpose:* turn behaviour into a regression-testable artifact.
*Affected:* `crates/crpg-testkit`.
*Dependencies:* T8b.
*Work:* a `.replay` format (seed, campaign id and version, engine version, ordered `(tick, input)` list), record/playback through public simulation APIs, comparison against a golden hash file, and an exact-tick structured divergence report.
*Test:* portable record/playback behavior tests and exact-tick golden divergence. T009c/ADR-0012 correct the original Linux-only selection to independent Windows/MSVC and Linux/GNU compile-time comparisons (§16.1); a deliberate behaviour change must fail the affected target's gate.
*Done when:* replay semantics are verified and CI step 9 performs real scoped comparisons without skipped placeholders. T009a's implementation remains uncommitted; its historical Linux verification is retained, not claimed as T009c verification.

**T9c (T009c). Windows-primary platform correction (`crpg-testkit`)**
*Purpose:* gate the primary Windows authoritative runtime while retaining fully supported Linux headless regression coverage (ADR-0012).
*Affected:* `crates/crpg-testkit` only for code, plus the policy/artifact/task scope listed in `tasks/T009c.md`.
*Dependencies:* T9a; intentionally before T9b despite the suffix.
*Work:* independent native target-scoped goldens, compile-time real comparisons, canonical LF replay/golden artifacts, and documentation alignment. Preserve portable tests and exact comparison semantics; no server, loader, packaging, or placeholder product CI implementation.
*Test:* genuine Windows/MSVC and Linux/GNU generation/verification under pinned Rust 1.98.0, normal test profile, and default features, including missing-baseline failure proofs and the complete native gates/provenance record in `tasks/T009c.md`.
*Done when:* both supported-target gates and all required verification pass with reviewed provenance. **Current status: implementation/verification complete in the working tree awaiting review/merge; final audit complete.** All required gates passed on native Windows/MSVC and genuine Linux/GNU in WSL Ubuntu 24.04; see the [completion record](../tasks/T009c.md). T009a is also uncommitted and T009c remains the priority before T009b; neither task is merged.

**T9b. Replay CLI (`crpg-cli`)**
*Purpose:* expose T9a's replay behavior without duplicating it in the binary.
*Affected:* `crates/crpg-cli`.
*Dependencies:* T9a and T009c; blocked until T009c's platform correction and verification are complete.
*Work:* `crpgc replay` loads a replay and its scoped golden through the T9a API, exits successfully on equality, and prints the structured divergence on failure. It inherits ADR-0012's exact-build target scope, not the superseded Linux-only policy; it adds no cross-platform equality or replay semantics.
*Test:* CLI success, divergence, malformed replay, and missing-file cases assert exit status and diagnostics.
*Done when:* the CLI is a thin consumer of T9a and no replay semantics live in `crpg-cli`.

(T9a/T9b are spec §24's original T9, split per E004 and sequenced by E020;
T009c/ADR-0012 insert the corrective platform gate between them: one task,
one crate.)

**T10. `crpg-data`: schema types, canonical writer, loader**
*Purpose:* the campaign format.
*Affected:* `crates/crpg-data`, `schemas/`.
*Dependencies:* T6.
*Work:* Rust types for `Campaign`, `World`, `Area`, `Creature`, `Item`, `Dialogue`, `Quest`, `Faction`, `Placement`, explicit aggregate documents, package ids, `campaign.lock`, and `assets.lock`; `schemars` generation into `schemas/`; the canonical JSON writer; deterministic flat dependency resolution; lockfile read/write APIs with the E016 authority split; the loader building the object-ULID `id → (type, path)` index; event-IR graph types (`Trigger`/`Node`/action signatures), including tick-count `Wait`, per ADR-0008.
*Test:* round-trip property tests; canonical-writer idempotence; schema drift check in CI.
*Done when:* the `one_area_one_creature` fixture loads and re-serializes byte-identically.

**T11. Validation and diagnostics**
*Purpose:* the feature that makes the editor and the agent workflow both work.
*Affected:* `crpg-data`, `crpg-cli`.
*Dependencies:* T10.
*Work:* `validate(&Campaign) -> Vec<Diagnostic>` with file, JSON pointer, severity, code, message, and optional suggested fix. Checks: dangling references, duplicate ids, duplicate slugs, unreachable dialogue nodes, quests with no completion path, missing assets, missing locale keys. `crpgc validate --json`.
*Test:* the `broken_references` fixture produces exactly the expected diagnostic set, compared as a snapshot.
*Done when:* diagnostics are stable, positioned, and machine-readable.

**T12. Migration framework**
*Purpose:* guarantee that content authored today loads in five years.
*Affected:* `crpg-data/src/migrations`, `crates/crpg-data/tests/fixtures`.
*Dependencies:* T10.
*Work:* per-type version chains of pure `Value → Value` functions; loader applies them in memory; `crpgc migrate` rewrites files.
*Test:* a `v1` fixture campaign migrates to current and matches a golden; CI fails if a schema version increments without a migration and a fixture.
*Done when:* the CI gate exists and has been proven by adding a dummy `v1 → v2`.

**T13. Scaffolding and introspection CLI**
*Purpose:* make the campaign format usable by AI agents without the editor.
*Affected:* `crpg-cli`.
*Dependencies:* T11.
*Work:* `crpgc new <type> --slug <s>`, `crpgc schema <type>`, `crpgc explain <id>` (object plus inbound/outbound references), `crpgc fmt`, `crpgc lock` as a thin caller of T10's resolver/lock writer, and the `crpgc run --ticks N --hash-every M` wrapper over the T008b harness (transferred from T008 per E004, 2026-09-06).
*Test:* the literal acceptance test from Phase 2 — give an LLM only `crpgc schema creature` output and have it produce a file that passes `crpgc validate` on the first attempt. Record the transcript.
*Done when:* that test passes for creature, item, dialogue, and quest.

**T14. `crpg-rules`: stats and the modifier pipeline**
*Purpose:* the highest-risk component, built and tested first.
*Affected:* `crates/crpg-rules`.
*Dependencies:* T6, T10.
*Work:* `StatBlock`, `Modifier`, `ModTypeId`, stacking policies as ruleset data, `query(entity, stat, context) -> (Value, ModifierBreakdown)`, derived stats with cycle detection at load, kernel hook event types (`BeforeRoll`, `OnDeath`, …) per ADR-0008, and `StatBlock`'s persisted string conversion pair plus its round-trip test per ADR-0006 D4 (E014).
*Test:* a 100+ case table-driven suite; property tests for order-independence within a stacking group and for exact reversal on effect removal.
*Done when:* every query returns a breakdown, and the `AGENTS.md` invariant "no game-system knowledge" holds under review.

**T15. Dice, outcome tables, and resolution**
*Purpose:* the second half of the rules kernel.
*Affected:* `crpg-rules`.
*Dependencies:* T14.
*Work:* `DiceExpr` parser and evaluator (`2d6+3`, `4d6kh3`, `1d20`), `OutcomeTable` as data, `ResolutionRequest`/`Outcome`, `ResourcePool` with refresh triggers.
*Test:* dice distribution tests with a fixed seed; PF2e-style and 5e-style outcome tables both evaluated correctly from data alone; resource refresh timing.
*Done when:* no `d20` literal exists anywhere in `crpg-rules`.

**T16. `rulesets/minimal-d6` and a headless combat**
*Purpose:* the abstraction test, and the first end-to-end proof that data drives the engine.
*Affected:* `rulesets/minimal-d6`, `crpg-sim` (timeline, turn order, damage application), `campaigns/fixtures/combat_basic`.
*Dependencies:* T15, T9.
*Work:* declare three attributes, HP, one attack ability, a two-band outcome table, one action per turn. Build the timeline and turn system. Run a two-creature fight headlessly from a replay.
*Test:* `crpgc replay combat_basic.replay` produces a stable hash sequence and ends with one creature dead.
*Done when:* the fight runs and no code in `crpg-rules` or `crpg-sim` mentions anything specific to this ruleset.

**T17. `rulesets/srd-lite` — the abstraction is proven or it is not**
*Purpose:* discover kernel leaks now rather than in year two.
*Affected:* `rulesets/srd-lite` only, ideally.
*Dependencies:* T16.
*Work:* six attributes, AC, HP, levels 1–2, a d20 outcome table with natural-20 crits, and two action pools (action, bonus action).
*Test:* the same fixture campaign runs under both rulesets by swapping one manifest line.
*Done when:* it works **and** the diff to `crpg-rules` is empty. If the diff is not empty, stop, read what you had to change, and redesign the kernel before proceeding. This is a hard gate.

**T18. `crpg-net` protocol v1 and the simulated-network transport**
*Purpose:* start the networking layer against a testable transport before real sockets.
*Affected:* `crates/crpg-net`, `crpg-contracts` (the `Transport` trait), `crpg-testkit`.
*Dependencies:* T16.
*Work:* message enums for `ClientIntent` and `DeltaOp`, `postcard` codec with a version byte, `Transport` trait, an in-memory transport with configurable latency, jitter, loss, and reorder, plus `assert_transport` conformance tests.
*Test:* codec round-trip property tests; conformance suite passes for the in-memory transport; a two-`World` desync test over 5,000 ticks with 3% loss.
*Done when:* the quinn transport (added next) can drop into the same trait with no changes above it.

---

After T18, the critical path splits and parallel agent work becomes genuinely productive: one track takes networking to real QUIC and a running server, one track builds the client bridge, one builds `crpg-edit`'s document and command model. All three depend only on things that now exist and are tested.

---

## 25. Closing assessment

The two things most likely to determine whether this project succeeds are not technical choices in the usual sense.

The first is **T17**. If the rules kernel survives a second, structurally different ruleset without modification, the ambitious part of the vision is achievable. If it does not, you will find out in month four rather than month thirty, and the redesign costs a fortnight rather than a year.

The second is **whether you can keep the editor small**. Every CRPG toolkit project that has died, died in the editor. The defences in this plan are generated property forms, headless-first command APIs, and the mandatory list in Section 22. Use them ruthlessly.

Everything else in this document is recoverable. The Godot decision is reversible by construction. The networking model can be extended. The campaign format can be migrated. The rules kernel and the editor's scope are the two places where being wrong is expensive, which is exactly why they are addressed in Phase 3 and Phase 6 rather than later.

---

## Agent log

- 2026-09-05 (UTC) · opencode/muse-spark + E002/E008 · §2.4 now references the single core EntityId per ADR-0006 instead of redefining it; §5.2 event-graph budget is instruction/bytecode, not wall-clock, preserving replay determinism. Dated inline notes mark both corrections.
- 2026-09-05 (UTC) · opencode/muse-spark + E001/ADR-0008 · Layer diagram, repo tree, and §16.2 test line now say generic event queue/substrate in core; SimEvent lives in sim, IR types in data, hooks in rules.
- 2026-09-06 (UTC) · opencode/muse-spark + E006-A/E009/E014/E015 · E009: World sketch owns `EventQueue<SimEvent>`, core diagram/tree lines say generic substrate, §24 T7/T10/T14 mirror the ADR-0008 assignments. E014: World serde is skeleton-only; interned boundaries persist strings via T014's conversion pair. E015: prediction is a buffer outside sim, `Timeline` is `BTreeMap<(InitiativeKey, EntityId)>` with the container in T007 and advance rules in T008.
- 2026-09-06 (UTC) · opencode/muse-spark + E004 decided (Option A) · §24 T8 is now T8a (sim: `state_hash`, tick loop, advance) + T8b (testkit: hash harness, first testkit code); the `crpgc run` wrapper moved to T13. Rule itself unchanged.
- 2026-09-06 (UTC) · opencode/gpt-5.6-sol + E020 decision · Made gates 7–13 capability-gated instead of skipped placeholders and split T9 into testkit replay semantics plus a later CLI wrapper; T9a activates the real canonical-Linux golden gate in the existing test job.
- 2026-09-06 (UTC) · opencode/gpt-5.6-sol + E016 decision · Defined entity versus aggregate JSON documents, separated object ULIDs from package ids, assigned one authority to each lock/manifest stage, made IR waits tick-based, and assigned data behavior to T10 with a thin T13 lock CLI.
- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c documentation alignment · Aligned process/server/editor architecture, replay testing and gates 9/12/13, packaging/extensibility, technical decisions, roadmap, and final recommendation with ADR-0012's Windows-primary/Linux-supported policy. Recorded future host/loader/product-gate obligations without implementation and kept T009a uncommitted and T009c in progress pending native verification; prior attribution and Linux provenance remain intact.
- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c verification status and audit · Replaced active pending-verification wording with the reported passing native gates and completion-record links, keeping review/merge and final audit outstanding. Corrected replay reporting to identity plus structured mismatch with semantic diff deferred, and linked the open planning-only E023 task without claiming loader implementation.
- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c final audit · Updated active gate and roadmap status to the reported completed final audit and working-tree implementation/verification completion, retaining completion-record links. Review/merge remains outstanding, T009a is also uncommitted, and T009c retains priority before T009b without changing platform policy.
