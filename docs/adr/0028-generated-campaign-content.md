# ADR-0028: Keeping the engine open to LLM-generated campaign content

Date: 2026-10-18 (UTC)
Status: **Proposed**. Filed at the user's request to explore the idea; nothing
is approved, scheduled or built. Each "Decision" item below is a proposal
until the user accepts it.

## Context

The user wants to explore, as a long-term direction, a mode in which a large
language model (LLM) acts as a game master. It would invent an overarching
story while the game runs, introduce characters when the story needs them,
and let the player talk to those characters in free text. Tabletop players
already do this with general-purpose chat models. The user also stated the
priority. **Human-authored campaigns stay the main product.** This ADR is
here so that today's work does not close the door on generation. It does not
start building it.

These facts are true of the engine today, and they shape the proposal:

- The campaign format was designed for machine authoring from the start
  (spec §4.1 ranks "machine-generatable by an LLM" first). The literal
  schema-only LLM trials in `tasks/T013.md` showed that a model can write
  valid creature, item, dialogue and quest documents.
- `crpg_data::load_campaign` takes documents as in-memory bytes and does no
  I/O, and `validate` returns positioned diagnostics. A runtime generator can
  call the same loader and validator that `crpgc validate` uses.
- Dialogue (`DialogueBody`), quests and the event IR are data. Their effects
  go only through `ActionCall` against the registered action vocabulary
  (`ActionSignatureStore`, T029a). The IR already has a `Trigger::Custom`
  hook.
- Simulation is replay-deterministic on the exact build (ADR-0009,
  ADR-0012). An LLM call is not deterministic, not bounded in time, and may
  not be available.
- Authority is server-side, including the embedded single-player server.
  Clients receive filtered data, never logic or hidden state (spec §4.7,
  §12.1).
- Spec §22 item 5 postpones "random/procedural dungeon or terrain
  generation". This proposal is about narrative and character content, not
  level geometry, and it postpones the work too (see Consequences).

## Decision

Proposed guardrails. Each one is cheap to keep today and expensive to
retrofit later.

1. **Generated content is campaign data. It is not a second runtime.** A
   generator (the "director") outputs the same canonical documents a human
   author writes, such as `crpg.creature/1`, `crpg.dialogue/1`,
   `crpg.quest/1` and `crpg.locale/1` documents. These go through the same loader
   and validator. No document type, interpreter or rules path exists only
   for generated content. A side effect is useful: the generator can also
   run at authoring time, in the editor or through `crpgc`, to produce
   drafts that a human edits and commits.

2. **The model call happens outside the deterministic core. Its accepted
   output is a recorded input.** No LLM client, prompt, network call or
   model output reaches `crpg-core`, `crpg-rules` or `crpg-sim`, except as
   validated documents. The accepted output enters the authoritative host
   the way a player intent does. It is drained at tick step 1 (spec §10),
   and the replay log and save record it as document bytes plus their hash.
   Replay feeds back the recorded documents and never calls a model again.
   Exact-build replay determinism therefore holds without any change to
   ADR-0009.

3. **Generation never blocks a tick.** Requests are asynchronous. The game
   always has an authored or rule-based fallback for the case where no
   result arrives, such as a holding line, a "come back later", or an
   authored stock quest. "No model configured" is a supported, fully
   playable state, and it is the default.

4. **Proposals are untrusted input and are validated like a hostile
   client's.** Before anything is accepted, the host checks the schema,
   references and action signatures, and then the campaign's own limits.
   Generated content gets a trust level narrower than T1 (spec §12.1).
   - It is data only. The director may not produce Lua.
   - Event IR and dialogue may call only the actions that the campaign
     allows for generated content.
   - It may reference authored objects but may never modify or delete them.
   - It is subject to campaign-declared budgets on count, size and reward.

   A rejected proposal is a normal outcome. The host may retry with the
   diagnostics, as `crpgc validate --json` already lets agents do, or it
   falls back.

5. **Authored canon wins, and generation is opt-in per campaign.** A
   campaign that declares nothing generates nothing. A campaign that opts in
   declares, as data, where generation is allowed: *slots*, such as a side
   quest at a named hook, an extra character in an area, or free
   conversation with an NPC. It may also include a *lore bible* (facts,
   tone, names, things that must never happen) and a set of *protected*
   objects. The director writes inside those slots and nowhere else.

6. **Generated documents live in a per-save overlay.** They get fresh object
   ULIDs, their own locale table and their own provenance (model, campaign
   slot, tick). They are stored with the save, never in the authored
   package. An overlay can be exported as an ordinary campaign folder, so an
   author can curate the good content and promote it into authored canon
   through the normal editor and review path.

7. **Free-text conversation shows text, and its effects are typed
   actions.** When this is designed, a future dialogue node kind such as
   `freeform` will carry a persona, a knowledge scope, the actions it
   allows, and authored exit nodes. The model's reply has two parts:
   - The NPC's line, which is shown to the player and never changes state.
   - Zero or more proposed `ActionCall`s, validated against that node's
     allow-list. Examples are starting a quest, changing attitude or ending
     the conversation.

   The player's typed text is an intent like any other.

8. **The prompt counts as client-visible.** Anything put into a prompt may
   come back in text the client can read. Prompts are therefore built from
   the speaking NPC's knowledge scope and the player-visible state only.
   Hidden DCs, trap locations, secrets the party has not learned and other
   players' private state are not included. This is spec §4.7's
   server-side filter, applied to prompt construction.

9. **The provider is configured by the operator and is never part of a
   campaign.** The endpoint, which may be a local model or a hosted API, its
   credentials and its spending limits belong to the host or operator
   configuration. They never come from a campaign package and are never
   sent to clients. In multiplayer, the dedicated server's operator owns the
   configuration. Both Windows embedded and Linux headless hosts must
   support it (ADR-0012), with no Godot dependency.

10. **Placement and dependencies are decided when the work starts, not
    now.** The director will be either a new crate above `crpg-sim`, at the
    `{net, ai, script}` level or between that level and `crpg-server`, or an
    out-of-process service behind a narrow protocol. Either choice needs its
    own `ALLOWED` row in `tools/lint/deps.py` and separate dependency
    approval. This ADR adds no crate, dependency or code.

## Authority and affected surface

- Decided by: not decided. Proposed for the user's review.
- Authoritative contract: none yet. A future task file will own the slot
  schema, the overlay format, the `freeform` dialogue node and the director
  interface.
- Owning crate(s)/files: none changed. Future work would touch `crpg-data`
  (slot, lore and `freeform` schemas, and overlay loading), `crpg-server`
  (accepting proposals as recorded inputs), `crpg-persist` (storing the
  overlay in saves) and a new director crate or service. Under "one task =
  one crate" that becomes several tasks.

## Compatibility, hash and dependency impact

- Persisted/serialized shapes: unchanged. A later overlay and new schema
  versions would follow spec §4.5's migration rules like any other format
  change.
- Replay, `state_hash`, goldens: unchanged. Item 2 exists so that they stay
  unchanged once generation exists.
- Dependency graph and external dependencies: unchanged (item 10).
- Platform scope (ADR-0012): no change now. Item 9 states the future
  requirement.

## Alternatives considered

- **Call the model from inside the simulation, for example from an IR node
  or from Lua.** Rejected. It breaks replay determinism and blocks ticks,
  and it would give an unbounded, network-dependent component authority over
  state.
- **Let the model drive state directly**, by narrating outcomes or awarding
  items as text. Rejected. Rules would no longer be enforced by the rules
  kernel, and nothing could be validated, saved as data or replayed.
  Item 7's split between text and actions keeps the kernel authoritative.
- **Let the model write Lua.** Rejected for now. Generated code is the
  highest-risk path through the sandbox (spec §21 risk 12) and gives no
  narrative ability that data plus registered actions lacks. It could be
  revisited with evidence.
- **A separate "AI campaign" format or runtime.** Rejected. It would split
  the format, the tools and the tests, and generated content could never be
  promoted into authored campaigns.
- **Do nothing until the feature is built.** Rejected as the only response.
  Most of the guardrails above are things to avoid breaking, such as the
  I/O-free loader, data-only effects and server-side filtering. Writing them
  down now is how reviews can catch a change that would close the door.

## Consequences

- Easier: the feature can be added later as a layer that consumes the
  existing format, validator, action registry and authority boundary,
  without reworking the core. The same director also works as an authoring
  assistant for human designers. That use has lower risk and could come
  first.
- Harder: some future data features need one more check, namely whether
  they still work when the content comes from an overlay instead of the
  authored package. Examples are locale keys, quest state and references to
  generated ULIDs.
- Deliberately left open: the slot and lore-bible schemas, the overlay
  format, the `freeform` node, the director's placement, provider support,
  cost controls, content moderation, and how multiplayer parties share one
  generated story. The work belongs after the dialogue and quest depth of
  Phase 8 (spec §18). At the earliest it fits as an optional track beside
  Phase 9 AI, and it should not displace authored-content work.
- Review prompt for current work: a change that makes the loader need the
  filesystem, adds an effect path outside `ActionCall`, or sends unfiltered
  state toward clients would break items 1, 4 or 8. It should be flagged,
  even though nothing here is accepted yet.

## Acceptance and evidence

Not yet run. Nothing is implemented. If the user accepts this ADR, acceptance
means recording that decision and citing this ADR from spec §4.8 and §5.3.
The first implementing task would then define its own tests. At minimum,
replay of a session with generated content must not call any model, and
hostile proposals must be rejected.

## Supersession and corrections

## Agent log

- 2026-10-18 (UTC) · claude-code + user request (LLM-generated campaigns) · Filed as Proposed to record the guardrails that keep runtime and author-time LLM generation possible without touching determinism, authority or the human-authored priority. No code, crate or dependency changes.
