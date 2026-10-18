# ADR-0028: A generative game mode, and keeping the engine open to it

Date: 2026-10-18 (UTC)
Status: **Proposed**. Filed at the user's request to explore the idea; nothing
is approved, scheduled or built. Each "Decision" item below is a proposal
until the user accepts it.

## Context

The user wants a long-term **game mode** in which a large language model
(LLM) acts as the game master while the game is played. It would invent an
overarching story as play goes on, introduce characters when the story needs
them, let the player talk to them in free text, and write the content and
the **Lua scripts** that make all of this work. Tabletop players already do
this with general-purpose chat models. The user also set the priority.
**Human-authored campaigns remain the main product.** In the editor,
generation must be strictly opt-in, so that nothing generated enters a
human-authored campaign unless the author asks for it.

These facts are true of the engine today, and they shape the proposal:

- The campaign format was designed for machine authoring from the start
  (spec §4.1 ranks "machine-generatable by an LLM" first). The literal
  schema-only LLM trials in `tasks/T013.md` showed that a model can write
  valid creature, item, dialogue and quest documents. Spec §5.4 chose Lua
  partly because LLMs write it fluently.
- `crpg_data::load_campaign` takes documents as in-memory bytes and does no
  I/O, and `validate` returns positioned diagnostics. A runtime generator can
  call the same loader and validator that `crpgc validate` uses.
- Dialogue (`DialogueBody`), quests and the event IR are data. Their effects
  go only through `ActionCall` against the registered action vocabulary
  (`ActionSignatureStore`, T029a; trusted handlers in `crpg-script`, T029b).
  The IR already has `Trigger::Custom` and `CallScript` hooks.
- The Lua sandbox is **designed but not built**. Spec §5.4 and ADR-0005
  specify it: no `io`/`os`/`load`/`require`/`debug`, the deterministic sim RNG
  in place of `math.random`, ordered `pairs`, instruction and memory budgets,
  and one generated binding module. `crpg-script` currently has the action
  bindings only, with no Lua runtime (`docs/architecture/crpg-script.md`).
- Simulation is replay-deterministic on the exact build (ADR-0009,
  ADR-0012). An LLM call is not deterministic, not bounded in time, and may
  not be available.
- Authority is server-side, including the embedded single-player server.
  Clients receive filtered data, never logic or hidden state (spec §4.7,
  §12.1).
- Spec §22 item 5 postpones "random/procedural dungeon or terrain
  generation". The proposal below keeps to that: the mode generates story,
  characters, dialogue, quests and scripts, and it assembles places from
  human-made parts instead of generating geometry or art.

## Decision

Proposed. Items 1–3 describe the mode, and items 4–12 are the guardrails
that make it possible without weakening the core.

### The mode

1. **Generative play is a game mode, chosen at new game.** Playing an
   authored campaign is one way to start a game, and starting a generative
   game is another. The mode, its premise and its seed are recorded in the
   save. A generative game is built from:
   - a **ruleset**, the same packages as today;
   - one or more **kits**, which are ordinary human-made packages of areas,
     prefabs, creatures, items, assets and optionally a lore bible, that the
     director composes from;
   - a **premise** written by the player, such as tone, setting or "a
     smuggler's revenge story";
   - a **director**, the server-side service that runs the model.

   An authored campaign can also be used as a kit, or as the starting
   point that the director continues from. Kits are where human content
   stays central in this mode.

2. **The director plans and fills in content as play goes on.** It keeps a
   story plan (arcs, factions, open threads) as generated data, and it adds
   content when play reaches somewhere it hasn't filled yet: a new
   area visit, a quest resolution, a rest, a conversation. Characters,
   dialogue, quests, encounters, event graphs and Lua scripts are all
   generated in this way.

3. **The editor never generates anything unless the author opts in.** An
   authored campaign generates nothing, and adds no generated content to
   its package, unless its author opts in:
   - **At authoring time**, the director is an editor assistant that writes
     drafts. Drafts are marked as generated and are never saved into the
     campaign until the author accepts them. After that they are authored
     content, reviewed and committed like any other change.
   - **At play time**, a campaign may declare **slots**, such as a side
     quest at a named hook, an extra character in an area, or free
     conversation with one NPC, plus *protected* objects that generation
     may reference but never change. Without slots, the director does not
     run in that campaign. These are the only places runtime generation
     touches an authored campaign.

### The guardrails

4. **Generated content uses the formats a human author uses. It is not a
   second runtime.** The director writes the canonical documents a human
   writes, such as `crpg.creature/1`, `crpg.dialogue/1`, `crpg.quest/1`,
   `crpg.graph/1` and `crpg.locale/1`, plus `scripts/lua/*.lua` source.
   The same loader, validator and Lua sandbox check all of it. No document
   type, interpreter or rules path exists only for generated content.

5. **The model call happens outside the deterministic core. Its accepted
   output is a recorded input.** No LLM client, prompt, network call or
   model output reaches `crpg-core`, `crpg-rules` or `crpg-sim`, except as
   validated documents and script source. The accepted output enters the
   authoritative host the way a player intent does. It is drained at tick
   step 1 (spec §10), and the replay log and save record it as bytes plus
   their hash. Replay feeds the recorded content back and never calls a
   model again. Generated Lua runs under the same deterministic sandbox as
   authored Lua, so a replay re-runs the recorded scripts with identical
   results. Exact-build replay determinism therefore holds without any
   change to ADR-0009.

6. **Generation never blocks a tick.** The director works ahead of the
   player where it can, for example on the next area or the next likely
   conversation. Every request has a fallback for the case where no result
   arrives in time, such as a holding line, a "come back later", or kit
   content. A save made in generative mode stays loadable and replayable
   without a model. It can still be played, using fallbacks, until a model
   is available again.

7. **Generated Lua is allowed, and treated as hostile code.** In this mode,
   the player's own free text goes into the prompts that produce scripts,
   so generated Lua must be assumed to be written by the player, or by
   someone the player copied text from. The defence is the spec §5.4
   sandbox, which is enforced, not trusted:
   - Generated scripts run in the same server-only sandbox, with the same
     removed libraries, deterministic RNG, ordered `pairs`, and
     instruction and memory budgets as authored Lua. There is no extra
     power for generated code.
   - They also get a **narrower binding surface**. The generated-content
     action allow-list (item 8) applies inside Lua as well as in the IR, so
     a script cannot reach an action that a generated document could not
     call.
   - The host compiles every script before accepting it and rejects it on
     any compile error, missing entry point, or call outside the binding
     module. A script that exceeds its budget at run time aborts as spec
     §5.2 describes and is reported to the director.
   - This makes the sandbox the main security boundary of the mode. Spec
     §21 risk 12, the sandbox escape risk, is therefore a hard prerequisite.
     The generative mode cannot ship until the sandbox exists and its
     escape tests include adversarial, model-written code.

8. **All proposals are untrusted input and are validated like a hostile
   client's.** Before acceptance the host checks the schema, references and
   action signatures, compiles the Lua as in item 7, and then applies the
   mode's or campaign's own limits:
   - Generated content may reference authored and kit objects, but may
     never change or delete them. It may change generated objects.
   - Event IR, dialogue and Lua may call only the actions on the
     generated-content allow-list, which the kit or the campaign declares.
   - Budgets apply to count, size, reward and script cost.

   A rejected proposal is a normal outcome. The host may return the
   diagnostics to the director so it can retry, as `crpgc validate --json`
   already lets agents do, or it falls back (item 6).

9. **Generated content lives in a per-save overlay.** Generated documents
   and scripts get fresh object ULIDs, their own locale table, and their own
   provenance: model, prompt reference, tick and the slot or story beat that
   asked for them. They are stored with the save, never in a kit or an
   authored package. An overlay can be exported as an ordinary campaign
   folder, so a good generated playthrough can become the starting draft of
   a human-authored campaign. Promoting it goes through the editor's
   opt-in path (item 3).

10. **Free-text conversation shows text, and its effects are typed
    actions.** A future dialogue node kind such as `freeform` will carry a
    persona, a knowledge scope, the actions it allows, and its exit nodes.
    The model's reply has two parts:
    - The NPC's line, which is shown to the player and never changes state.
    - Zero or more proposed `ActionCall`s, or calls into accepted scripts,
      validated against that node's allow-list. Examples are starting a
      quest, changing attitude or ending the conversation.

    The player's typed text is an intent like any other. Rules outcomes,
    such as checks, damage and loot, always come from the rules kernel,
    never from the narration.

11. **The prompt counts as client-visible.** Anything put into a prompt may
    come back in text the client can read. Prompts for dialogue are
    therefore built from the speaking NPC's knowledge scope and the
    player-visible state only. Hidden DCs, trap locations, secrets the party
    has not learned and other players' private state are not included.
    The story planner may see more, but what it writes is subject to the
    same server-side filtering as any authored content before a client
    receives it (spec §4.7).

12. **The provider is operator configuration, and placement is decided
    later.** The endpoint, which may be a local model or a hosted API, its
    credentials and its spending limits belong to the host or operator
    configuration. They never come from a kit or campaign and are never
    sent to clients. In multiplayer, the dedicated server's operator owns
    the configuration. Both Windows embedded and Linux headless hosts must
    support it (ADR-0012), with no Godot dependency. The director will be
    either a new crate above `crpg-sim` or an out-of-process service behind
    a narrow protocol. Either choice needs its own `ALLOWED` row in
    `tools/lint/deps.py` and separate dependency approval. This ADR adds no
    crate, dependency or code.

## Authority and affected surface

- Decided by: not decided. Proposed for the user's review.
- Authoritative contract: none yet. Future task files will own the kit
  manifest, the slot and lore-bible schemas, the overlay format, the
  `freeform` dialogue node, the generated-content allow-list and the
  director interface.
- Owning crate(s)/files: none changed. Future work would touch:
  - `crpg-data`: the kit, slot, lore and `freeform` schemas, and overlay
    loading;
  - `crpg-script`: the Lua sandbox, which is needed regardless, and the
    narrowed binding surface;
  - `crpg-server`: the game mode, and accepting proposals as recorded
    inputs;
  - `crpg-persist`: storing the overlay in saves;
  - `crpg-edit` and `crpg-godot`: the editor opt-in and the draft workflow;
  - a new director crate or service.

  Under "one task = one crate" that becomes several tasks.

## Compatibility, hash and dependency impact

- Persisted/serialized shapes: unchanged. A later overlay, kit manifest and
  new schema versions would follow spec §4.5's migration rules like any
  other format change.
- Replay, `state_hash`, goldens: unchanged. Item 5 exists so that they stay
  unchanged once the mode exists.
- Dependency graph and external dependencies: unchanged (item 12).
- Platform scope (ADR-0012): no change now. Item 12 states the future
  requirement.

## Alternatives considered

- **Call the model from inside the simulation, for example from an IR node
  or from Lua.** Rejected. It breaks replay determinism and blocks ticks,
  and it would give an unbounded, network-dependent component authority over
  state. The model writes the script, and the sandbox runs it later.
- **Let the model drive state directly**, by narrating outcomes or awarding
  items as text. Rejected. Rules would no longer be enforced by the rules
  kernel, and nothing could be validated, saved as data or replayed.
  Item 10's split between text and actions keeps the kernel authoritative.
- **Data only, with no generated Lua.** This was the first draft of this
  ADR. The user rejected it: the mode needs logic that the action
  vocabulary alone cannot express. Item 7 takes the risk on explicitly
  instead.
- **Give generated Lua its own, looser runtime**, for example to let it call
  the model or keep state between ticks. Rejected. It would be a second
  sandbox to secure and would break replay determinism.
- **A separate "AI campaign" format.** Rejected. It would split the format,
  the tools and the tests, and generated content could never be promoted
  into authored campaigns.
- **Procedurally generate geometry and art.** Rejected for now (spec §22
  item 5). Places are assembled from human-made kit parts.

## Consequences

- Easier: the mode can be added as a layer that consumes the existing
  format, validator, action registry, sandbox and authority boundary,
  without reworking the core. The editor assistant (item 3) uses the same
  director with lower risk, and could ship first.
- Harder:
  - The Lua sandbox now has to defend against adversarial scripts in normal
    single-player play, not only on public servers. Its escape tests and
    budgets need to be built with that in mind.
  - Some future data features need one more check, namely whether they still
    work when the content comes from an overlay instead of an authored
    package. Examples are locale keys, quest state, references to generated
    ULIDs, and scripts that refer to generated objects.
- Deliberately left open: the kit manifest, the slot and lore-bible schemas,
  the overlay format, the `freeform` node, how the story planner is
  represented, the director's placement, provider support, cost controls,
  content moderation, and how multiplayer parties share one generated
  story. The work depends on the Lua sandbox and on Phase 8's dialogue and
  quest work (spec §18). At the earliest it fits as an optional track
  beside Phase 9 AI, and it should not displace authored-content work.
- Review prompt for current work: flag any change that would break one of
  these items, even though nothing here is accepted yet:
  - the loader needing the filesystem (item 4);
  - an effect path outside `ActionCall` and the binding module (items 7
    and 8);
  - a Lua binding that is not deterministic or cannot be budgeted (items 5
    and 7);
  - unfiltered state sent toward clients (item 11).

## Acceptance and evidence

Not yet run. Nothing is implemented. If the user accepts this ADR, acceptance
means recording that decision and citing this ADR from spec §4.8, §5.4 and
§12.1. Each implementing task defines its own tests. At minimum:
- replaying a generative session never calls a model and reproduces its
  `state_hash`;
- hostile documents and hostile, model-written Lua are rejected or contained
  by the sandbox;
- an authored campaign without slots never invokes the director.

## Supersession and corrections

## Agent log

- 2026-10-18 (UTC) · claude-code + user request (LLM-generated campaigns) · Filed as Proposed to record the guardrails that keep runtime and author-time LLM generation possible without touching determinism, authority or the human-authored priority. No code, crate or dependency changes.
- 2026-10-18 (UTC) · claude-code + user feedback on the draft · Reframed as a generative game mode built from human-made kits, allowed generated Lua under the existing sandbox with a narrower binding surface, and made editor generation strictly opt-in, per the user's three corrections. Still Proposed; no code changes.
