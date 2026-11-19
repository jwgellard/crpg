# T101 authoring UX ADR — draft b (blind)

Date: 2026-11-19 (UTC)
Status: **Draft** — a blind draft written without reading any other draft of
this ADR. Nothing here is approved. A Claude `architect` writes the final ADR
(T101 is routed to the `architect`, `docs/IMPLEMENTATION_PLAN.md:301`); the
user accepts it (H-accept). This draft is set against the blind design T101b
and compared by the `reviewer` (`docs/IMPLEMENTATION_PLAN.md:301`).

## Status

Blind draft b. It was written from the fact sheet (`tasks/T101-facts.md`) and
the files named in the brief, without looking at the other lab's draft (T101b)
or any prior T101 draft. It is **not** accepted and binds no contract. The
`architect` owns the final ADR; this document is input to it, alongside T101b,
through the `reviewer`. M6-R's exit gate stands unchanged: "No M6 contract is
approved before then" (`tasks/DECISIONS-2026-10-16.md:108`).

## Context

T101 is the authoring UX ADR that gates every M6 editor contract
(`docs/IMPLEMENTATION_PLAN.md:301`). D28j made authoring a research stage
because "a major, major concern is both agent and human usability"
(`tasks/DECISIONS-2026-10-16.md:92`), and D28l fixed the audience: "Module
authors is the primary goal, yes with agents being equal. Your own game comes
after them" (`tasks/DECISIONS-2026-10-16.md:135`) and "Defining a ruleset is
an authoring task in its own right" (`tasks/DECISIONS-2026-10-16.md:136`).
This draft decides six things the T101 row names (`docs/IMPLEMENTATION_PLAN.md:301`):
personas, task flows for each §17.2 editor item and for creating a ruleset
from a template, the editor's information architecture, an intent-level
command layer above T058's `EditCommand`, the asset-import flow, and usability
acceptance criteria per M6 package.

What exists today, and what this ADR must build on without contradicting:

- **The edit API (T058).** `EditCommand` is a closed nine-variant enum, "the
  closed set of editor mutations. Every mutation goes through one of these"
  (`crates/crpg-edit/src/command.rs:15`): `CreateDocument`, `DeleteDocument`,
  `RenameDocument`, `SetValue`, `InsertValue`, `RemoveValue`, `DeleteObject`,
  `PlaceInstance`, `MovePlacement` (`crates/crpg-edit/src/command.rs:17-76`).
  `EditTarget` addresses a whole `Document(path)` or an `Object(id)` with a
  relative pointer (`crates/crpg-edit/src/command.rs:5`). `CampaignDocument`
  exposes `open`, `apply`, `apply_batch`, `undo`, `redo`, `validate`,
  `save_plan`, `mark_saved` (`crates/crpg-edit/src/document.rs:229-371`) and
  returns a `CommandReceipt { revision, changes, diagnostics }`
  (`crates/crpg-edit/src/document.rs:75`). `EditError` is 16 pinned variants
  (`crates/crpg-edit/src/error.rs:28`). IDs are caller-supplied
  (`crates/crpg-edit/AGENTS.md`, invariant 8), so no editor command can mint
  an id silently.
- **The CLI.** `crpgc` has eleven subcommands (`crates/crpg-cli/src/main.rs:110`)
  with the 0/1/2 exit-code contract (`docs/architecture/crpg-cli.md:72`).
  `crpgc apply` does not exist; T059 owns it and "its JSON format follows the
  T101 ADR" (`docs/IMPLEMENTATION_PLAN.md:316`). `crpgc new` scaffolds one
  typed document to stdout; `crpgc validate` and `crpgc explain` already exist.
- **The data.** `Campaign` and `EntryPoint` (`crates/crpg-data/src/document.rs:17,45`),
  `Dialogue`/`DialogueNode`/`DialogueBody` (`crates/crpg-data/src/document.rs:155,176,191`),
  `Quest`/`QuestState` (`crates/crpg-data/src/document.rs:239,260`), area
  triggers (`crates/crpg-data/src/document.rs:354`), the authored `EventGraph`
  (`crates/crpg-data/src/ir.rs:13`), and the asset authority `AssetsLock` /
  `AssetRecord` (`crates/crpg-data/src/package.rs:82,92`). Schemas are
  generated from Rust shapes — 17 stems today (`docs/architecture/crpg-cli.md:672`) —
  which is the substrate for generated property forms.
- **The bridge.** `crpg-godot` is the presentation bridge ("Godot presentation
  bridge (ADR-0033)", `crates/crpg-godot/src/lib.rs:3`); its only open inputs
  are a campaign dir, an encounter id, a side and a seed — "no document,
  command or asset call exists in the bridge" (`crates/crpg-godot/src/core/mod.rs:99`).
  An `apps/editor` project does not exist yet; the M6 `editor/` shell is T060
  (`docs/IMPLEMENTATION_PLAN.md:317`).
- **The spec's UX commitments.** Three distinguishing features: a continuous
  Problems panel, a References panel on every object, and property forms
  generated from schemas (`docs/CRPG_ENGINE_SPEC.md:1057-1059`). The main
  window is three IDE-like regions with none of Godot's vocabulary
  (`docs/CRPG_ENGINE_SPEC.md:1031`). The Phase 6 objective is "create the MVP
  campaign without hand-writing JSON" (`docs/CRPG_ENGINE_SPEC.md:1604`), and
  the definition of done is the §17.2 campaign "created entirely in the editor
  by someone who has never seen the JSON" (`docs/CRPG_ENGINE_SPEC.md:1606`).

The research findings this draft leans on are R8 (module-editor prior art),
R9 (narrative tools), R10 (agent-facing authoring), R11 (usability methods),
R12 (event/trigger authoring), R13 (ruleset authoring), R14 (pause and time
modes) and R15 (custom content), each in `docs/research/T099/`. The two
findings that shape everything else are R8 "Scaffold, never blank"
(`docs/research/T099/R8-deep.md:9`) and R10 "a few consolidated verbs above
`EditCommand`, shared by GUI and agents" (`docs/research/T099/R10-deep.md:12`).

## Personas

D28l fixes the ordering: module authors first, agents equal, then the studio's
own game (`tasks/DECISIONS-2026-10-16.md:135`). Each persona below gets a
name, a goal, what they author with, and what this ADR owes them.

1. **The module author** (primary). A person with RPG-modding skill but no
   engine or JSON knowledge: an NWN, Solasta or Foundry module author (spec
   risk 18, `docs/CRPG_ENGINE_SPEC.md:1785`). They define a ruleset, build an
   area, place creatures, write dialogue and a quest, and package it. They
   work through the GUI and through the same commands an agent uses, but they
   never see JSON or an `EditCommand`. They need: scaffolds, not blanks
   (R8 `docs/research/T099/R8-deep.md:9`); generated forms, not hand-written
   ones (`docs/CRPG_ENGINE_SPEC.md:1059`); a Problems panel that is the first
   thing polished (R8 `docs/research/T099/R8-deep.md:7`); and one prompted
   help facility (R8 `docs/research/T099/R8-deep.md:12`).
2. **The agent** (equal, not subordinate). An LLM driving the same authoring
   surface headlessly, per spec §11.2 "Agent-generated content cannot bypass
   invariants the GUI enforces" (`docs/CRPG_ENGINE_SPEC.md:1013`). It needs a
   small verb set with 3–4 sentence descriptions and eval-gated examples
   (R7, `docs/research/T099/R10-deep.md:42`), stable identity as its address
   rather than raw JSON pointers (R4, `docs/research/T099/R10-deep.md:27`),
   machine-readable errors with a pointer, code and suggested fix (R2,
   `docs/research/T099/R10-deep.md:17`), a reviewable plan step (R3,
   `docs/research/T099/R10-deep.md:22`), and receipts that carry changed paths
   and diagnostics only (R6, `docs/research/T099/R10-deep.md:37`). It is a
   first-class author, but R11 is explicit that LLM "users" are biased proxies
   and never proof of human usability (`docs/research/T099/R11-deep.md:15`).
3. **The studio (own game)** (later). The team building the product the engine
   exists to make. D28l says their game "comes after them"
   (`tasks/DECISIONS-2026-10-16.md:135`), so nothing in this ADR optimises for
   the studio's internal workflow at the authors' expense. They are served by
   the same headless-first order (D18) that keeps every GUI feature available
   to a script.
4. **The player** (downstream, not an author). They consume a packaged module
   and a ruleset-provided UI (`docs/architecture/crpg-data.md:364`). This ADR
   touches them only through its consequences: custom content must pass
   conformance so a player's client can load it (`docs/research/T099/R15-deep.md:5`),
   and authored modules must be safe to share (`docs/research/T099/R13-deep.md:19`).

The equal-status rule has a concrete shape: every authoring feature ships as a
headless command first, and the GUI is a caller of that same command surface —
not a second implementation. Spec §11.2 already says undo/redo "is one
implementation" and every mutation goes through `apply`
(`docs/CRPG_ENGINE_SPEC.md:1012`). This ADR extends that from `apply` up one
level: the intent layer (§ Intent command layer) is the one surface both
personas drive.

## Task flows

One flow per spec §17.2 item (`docs/CRPG_ENGINE_SPEC.md:1516-1527`) plus one
for creating a ruleset from a template. Steps name screens and `crpgc`
commands. Screen names are fixed in § Information architecture. Flows 6–12 are
runtime items; their flow is "author the prerequisite, then press Play and
observe", because the author's job for those items is to set up the content
the runtime consumes. `crpgc apply` and the intent verbs it carries are the
T059/T103 form and are quoted here as the target surface.

1. **Area with terrain and props (§17.2 item 1).**
   1. Explorer → right-click `Worlds` → "New area" (issues `create(area, …)`;
      the intent layer expands it to `CreateDocument`).
   2. The Area editor opens in the document area; the Properties panel shows
      the generated `area` form.
   3. Choose the Terrain tool; pick a flat base (default) or a heightfield;
      sculpt/paint a simple surface (T063; each brush stroke is a `set`
      command, not a new kind of mutation).
   4. Choose the Place tool; drop 2–3 props from the asset browser (each drop
      is a `place` command).
   5. Problems panel stays empty; if a prop id dangles it shows a diagnostic
      with a pointer into `placements.json`.
   6. `crpgc validate --json` reports `[]\n` (the clean canonical array,
      `docs/architecture/crpg-cli.md:444`).

2. **Player start, one NPC, one hostile creature (§17.2 item 2).**
   1. Area editor → Place tool → choose "Player start" from the spawn
      palette; drop it (a `place` of the entry-spawn placement).
   2. Explorer → `Creatures` → "New creature" (`create(creature, …)`); the
      Creature editor opens with a structurally valid scaffold.
   3. Repeat for the NPC and the hostile creature; set each one's appearance,
      faction and AI profile via generated form fields (`set` commands).
   4. Place both creatures in the area (a `place` each); the walkable overlay
      shows where they may stand.
   5. References panel on the area now lists both creatures as "placed in".
   6. Save (the editor calls `save_plan` then `mark_saved`,
      `crates/crpg-edit/src/document.rs:350,371`).

3. **Creatures use `minimal-d6` (§17.2 item 3).**
   1. Open a creature; its ruleset field defaults to the campaign's ruleset.
   2. Set it to `minimal-d6` (a `set` on the ruleset reference).
   3. The creature's stat block repopulates from the ruleset's stat
      declarations (`docs/architecture/crpg-data.md:146`): three attributes,
      HP, one attack, 2d6 resolution.
   4. The modifier breakdown for HP and the attack appears in the live rules
      panel (`docs/CRPG_ENGINE_SPEC.md:1065`).
   5. `crpgc explain <creature-id>` prints the same block headlessly, so the
      agent can verify what the author sees.

4. **Dialogue with three nodes and a quest-starting choice (§17.2 item 4).**
   1. Explorer → `Dialogue` → "New dialogue" (`create(dialogue, …)`); the
      Dialogue editor opens with a start node scaffold.
   2. Add two more nodes in the node graph (each is an `insert` on the nodes
      array); one is a `choice` with one option.
   3. Toggle the text view; the round trip shows the same three nodes as
      indented text and back, losslessly (`docs/CRPG_ENGINE_SPEC.md:663`).
   4. On the choice node, set the action to "start quest Q" (a `set` on the
      node's action; unresolved quest names flag in the Problems panel).
   5. Run the "walk this conversation" simulator against a test character;
      all three nodes reachable, none greyed (`docs/CRPG_ENGINE_SPEC.md:1067`).

5. **Quest with two states, completion on enemy death (§17.2 item 5).**
   1. Explorer → `Quests` → "New quest" (`create(quest, …)`); the Quest editor
      opens with `started` and `completed` states scaffolded.
   2. Set journal text per state (each a `set`).
   3. Add the transition `started → completed` and set its trigger to
      "enemy dies" — in the MVP this is the one hardcoded quest hook, per
      `docs/CRPG_ENGINE_SPEC.md:1531`.
   4. The reachability analysis shows `completed` reachable from the start
      (`docs/CRPG_ENGINE_SPEC.md:1068`).

6. **Press Play: in-process server plus client (§17.2 item 6).**
   1. Save (see flow 2 step 6).
   2. Press ▶ Play. The editor spawns an in-process server (`Session`) and
      connects a client viewport in the document area (T068).
   3. The console/playtest log shows the server and client handshake; the
      entity inspector attaches to the running session
      (`docs/CRPG_ENGINE_SPEC.md:1073`).

7. **Click-to-move over a baked navmesh (§17.2 item 7).**
   1. In the Area editor, bake the navmesh (T064); the walkable overlay
      renders it one key away (`docs/CRPG_ENGINE_SPEC.md:1064`).
   2. Press Play; click a ground point; the character pathfinds there with
      server authority and client prediction.
   3. The playtest log shows the server's acceptance of the move intent and
      the client's predicted path converging.

8. **Talk to the NPC, quest starts and journals (§17.2 item 8).**
   1. Press Play; click the NPC; the dialogue's first node plays.
   2. Choose the option; the quest moves to `started` and appears in the
      journal UI.
   3. The campaign debugger's quest-state watch confirms the transition.

9. **Attack, turn-based, real dice, combat log, enemy dies (§17.2 item 9).**
   1. Press Play; the hostile creature's encounter region is entered.
   2. Attack; the rules trace shows each roll and its inputs; the combat log
      prints the roll; the enemy's HP falls and it dies (the sim combat path,
      not an editor path).

10. **Quest completes, journal updates (§17.2 item 10).**
    1. After the enemy dies, the quest transition fires; the journal shows
       `completed`.

11. **Save, quit, load — state identical (§17.2 item 11).**
    1. Quit the editor; it saves (`save_plan`/`mark_saved`).
    2. Reopen; reload; the campaign bytes are canonical-identical (the editor
       writes through the canonical writer, `docs/CRPG_ENGINE_SPEC.md:450`),
       and Play reproduces the same world state.

12. **Separate `crpg-server`, two clients (§17.2 item 12).**
    1. From the editor, "Launch dedicated" exports the campaign and runs it
       against a separate `crpg-server` (T068's external path), with two
       clients tiled (multiplayer testing, `docs/CRPG_ENGINE_SPEC.md:1074`).
    2. Both clients see the same world; the author also verifies item 12
       headlessly via `crpgc run` and the M7 acceptance
       (`docs/IMPLEMENTATION_PLAN.md:336`).

13. **Create a ruleset from a template (D28l; T110, after M6).**
    1. Rules menu → "New ruleset from template" → choose `minimal-d6` or
       `srd-lite` as the base (R13 "clone-and-modify starters",
       `docs/research/T099/R13-deep.md:7`).
    2. The Rules editor opens with the template's stat declarations as a
       scaffold, not a blank.
    3. Add a stat, change an outcome table, patch one ability by identity
       (R13 layering and patch overrides, `docs/research/T099/R13-deep.md:9`).
    4. The overrides are stored as a diff against the base ruleset
       (`rules/overrides.json`, `docs/CRPG_ENGINE_SPEC.md:1070`), not a full
       copy.
    5. A headless dry-run checks the derived-stat expressions and migration
       idempotence before it is usable in a campaign (R13,
       `docs/research/T099/R13-deep.md:15`).

## Information architecture

## Intent command layer

## Asset import

## Usability acceptance criteria per M6 package

## Decisions

## Consequences

## Open questions (each with a recommendation)

## Agent log
