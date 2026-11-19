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

The main window is the spec's three IDE-like regions, with none of Godot's
vocabulary (`docs/CRPG_ENGINE_SPEC.md:1031`). The exact region layout is the
spec's diagram (`docs/CRPG_ENGINE_SPEC.md:1034-1052`); this section fixes the
names, the contents and the rules each region obeys.

**Top bar.** `Campaign ▾ Edit ▾ World ▾ Rules ▾ Test ▾ Package ▾` menu strip
plus the `▶ Play` / `⏹ Stop` transport. `Play` maps to T068's in-process
server-and-client; `Package` maps to the one packaging dialog
(`docs/CRPG_ENGINE_SPEC.md:1075`). The strip's vocabulary is fixed: no Godot
terms (`docs/CRPG_ENGINE_SPEC.md:1031`).

**Left — CAMPAIGN explorer.** A read-only tree (`docs/CRPG_ENGINE_SPEC.md:1676`)
over the campaign's directory layout: `worlds/`, `creatures/`, `items/`,
`dialogue/`, `quests/`, `factions/`, `scripts/`, plus `areas/<area>/` and the
`variables/`, `assets/`, `locale/`, `tests/` folders (`docs/CRPG_ENGINE_SPEC.md:480`).
The tree is derived from the document index, so every node is an object with a
stable id, not a file path the author sees. "New …" actions live here as
context-menu entries, each of which is a `create` intent. The tree is
read-only in the sense that dragging does not move documents (renames are
explicit), but selecting a node opens its editor.

**Centre — DOCUMENT AREA.** Tabs, one per open document; each tab is a typed
editor. The per-editor notes (`docs/CRPG_ENGINE_SPEC.md:1061-1074`) are the
source of truth for what each tab is; the MVP tabs are Area, Creature, Item,
Dialogue, Quest, and (post-M6) Rules. A tab is always a view of one document
or one aggregate (an area tab spans `area.json` plus its `placements.json`,
keeping the churny placements file separate, `docs/CRPG_ENGINE_SPEC.md:496`).
Play swaps the tab region into the connected client viewport.

**Right — PROPERTIES + REFERENCES.** PROPERTIES is generated from the document
schema, never hand-written (`docs/CRPG_ENGINE_SPEC.md:1059`); the generator is
the schema the data crate already emits (`docs/architecture/crpg-data.md:19`).
REFERENCES is the "what uses this / what does this use" panel on the selected
object, so deletion is safe because the blast radius is visible
(`docs/CRPG_ENGINE_SPEC.md:1058`).

**Bottom — PROBLEMS | CONSOLE | PLAYTEST LOG | SEARCH.** PROBLEMS is the
continuous validation panel, validated on every change like a compiler
(`docs/CRPG_ENGINE_SPEC.md:1057`); it renders `Vec<Diagnostic>`
(`docs/CRPG_ENGINE_SPEC.md:1678`), the same list `validate()` returns
(`crates/crpg-edit/src/document.rs:345`) and `crpgc validate` prints. A
diagnostic's pointer is a JSON pointer the author can click to jump.

**Two rules that make the regions coherent.**

1. **Every mutation is an intent, and every intent is undoable.** The GUI's
   widgets issue the intent verbs (§ Intent command layer); the document is
   never edited out-of-band. Undo/redo is one implementation through
   `apply`/`undo`/`redo` (`docs/CRPG_ENGINE_SPEC.md:1012`).
2. **The agent sees the same information the author sees, headlessly.** The
   explorer tree, the property schema, the references and the diagnostics all
   have `crpgc` forms (`schema`, `explain`, `validate`, `apply`), so an agent
   can navigate the identical structure without the GUI.

The on-disk layout is the editor's data model, not an implementation detail:
`campaign.json` is the manifest, `campaign.lock` is resolved
versions/checksums plus the `assets.lock` digest, `placements.json` stays
separate from `area.json`, `assets.lock` is path→BLAKE3→import-settings, and
every display text is a `locale/en.json` key
(`docs/CRPG_ENGINE_SPEC.md:482,485,496,506,508`). The editor writes through
the canonical writer and `crpgc fmt` enforces it
(`docs/CRPG_ENGINE_SPEC.md:450`).

## Intent command layer

A thin layer of consolidated verbs above T058's `EditCommand`, shared by the
GUI and `crpgc apply` (D28j: added "above it, not in its place",
`tasks/DECISIONS-2026-10-16.md:105`; R10 `docs/research/T099/R10-deep.md:12`).
It lives in `crpg-edit` (T103's first owner, `docs/IMPLEMENTATION_PLAN.md:303`),
so the GUI and the CLI call one implementation. It is **not** a second
document model: every verb compiles to one or more `EditCommand`s and runs
through `apply`/`apply_batch`, so the invariants, undo, caps and receipts are
T058's unchanged.

**Addressing (R4).** Identity is the agent-visible address: objects are named
by `Ulid`, and a field is a schema field name, not a raw JSON pointer. JSON
Pointers exist only for leaves a schema field cannot reach (R4,
`docs/research/T099/R10-deep.md:27`). The layer resolves an `(id, field)`
pair to `EditTarget::Object(id)` plus a relative pointer, using the same
index `crpg-edit` derives (`crates/crpg-edit/AGENTS.md`, "never trust the
opened index"). Edits are anchored (R5): a `set` names one uniquely-anchored
object; an ambiguous anchor fails, never guesses
(`docs/research/T099/R10-deep.md:32`).

**Errors (R2).** The layer adds an actionable error surface above `EditError`.
`EditError` is unchanged and is the last word on whether a command committed.
The layer's own errors are machine-readable `{ pointer, code, suggestion }`
triples (R2, `docs/research/T099/R10-deep.md:17`) for things `EditError`
cannot say: an unknown field name (with a "did you mean" against the schema),
a missing reference target, or an id that is not in the index. A structural
refusal stays an `EditError`; a semantic finding stays a diagnostic on the
receipt (`crates/crpg-edit/AGENTS.md`, invariant 7).

**Plan step (R3).** `plan` is a reviewable, saveable list of verbs that
`apply_plan` runs as one `apply_batch`. A plan never implies rollback: if it
fails it fails atomically and nothing is half-applied (R3,
`docs/research/T099/R10-deep.md:22`; T058 atomicity,
`crates/crpg-edit/AGENTS.md`, invariant 1).

**Receipts (R6).** Every verb returns the same `CommandReceipt` shape the GUI
uses — `revision`, changed paths, diagnostics
(`crates/crpg-edit/src/document.rs:75`) — so an agent's response is bounded
(R6, `docs/research/T099/R10-deep.md:37`).

The verbs. "Expands to" lists the `EditCommand`s; "Errors" lists the layer
errors plus the relevant `EditError` variants. IDs are always caller-supplied.

| Verb | Inputs | Expands to | Errors |
|---|---|---|---|
| `create` | `kind`, `slug`, `id` | `CreateDocument { path, document }` with a scaffolded document | `PathExists`, `BatchSize`, `InputTooLarge`; layer `unknown kind` |
| `delete` | `id` (or `path`) | `DeleteObject` (root object ⇒ `DeleteDocument`) | `UnknownObject`, `MissingPath` |
| `rename` | `from`, `to` | `RenameDocument` | `MissingPath`, `PathExists` |
| `set` | `id`, `field`, `value` | `SetValue { Object(id), pointer, value }` | `UnknownObject`, `WrongKind`, `Pointer`, `InputTooLarge`; layer `unknown field` |
| `set_at` | `path`, `pointer`, `value` | `SetValue { Document(path), pointer, value }` | `MissingPath`, `Pointer` |
| `insert` | `id`, `field`, `value` | `InsertValue { Object(id), … }` | `UnknownObject`, `WrongKind`; layer `field not an array` |
| `remove` | `id`, `field`, `index?` | `RemoveValue { Object(id), … }` | `UnknownObject`, `WrongKind`; layer `member not found` |
| `link` | `id`, `field`, `target_id` | `SetValue` whose value is the target `Ulid`, checked as a reference | `UnknownObject`, `WrongKind`; a dangling target is a receipt diagnostic, not an error |
| `place` | `area_id`, `placement` | `PlaceInstance` | `UnknownObject`, `WrongKind` |
| `move` | `placement_id`, `transform` | `MovePlacement` | `UnknownObject`, `WrongKind` |
| `plan` | `verbs[]` | none (returns a plan for review) | layer `invalid verb`, per-verb dry-run errors |
| `apply_plan` | `plan` | `apply_batch` over the plan's commands | `BatchSize`, the failing command's error with index |
| `validate` | — | `validate()` (read-only) | — |
| `undo` / `redo` | — | `undo()` / `redo()` | `NothingToUndo`, `NothingToRedo` |

Notes that pin the design:

- **`create` scaffolds, never blanks** (R8, `docs/research/T099/R8-deep.md:9`):
  the scaffold is a full, structurally valid document of the current schema
  tag, so a `create` alone never leaves the campaign invalid. `crpgc new`
  already builds one such document to stdout (`docs/architecture/crpg-cli.md:610`);
  `create` is that document installed by `CreateDocument`.
- **`link` is a first-class verb** because references are the one thing
  authors (and agents) get wrong, and the layer can turn a target id into a
  validated reference and report a dangling one on the receipt, where it
  belongs (invariant 7). The alternative — making every reference a raw
  `set` of an id string — is rejected in § Decisions.
- **The verb set is under twenty** (fourteen verbs here), with 3–4 sentence
  descriptions and eval-gated examples (R7,
  `docs/research/T099/R10-deep.md:42`). T103 pins the exact list and the
  `crpgc apply` JSON that encodes it; T059's JSON format "follows the T101
  ADR" (`docs/IMPLEMENTATION_PLAN.md:316`), so `apply` is a JSON encoding of
  these verbs, not of `EditCommand` directly.
- **The GUI issues the same verbs.** A form field edit is a `set`; a
  drag-drop placement is a `place`; the schema-derived property form is the
  GUI's spelling of `set`/`insert`/`remove`, so agent and human edits land on
  one transactional surface with receipts (R9,
  `docs/research/T099/R9-deep.md:55`).

## Asset import

Custom content is a D28l item: "the asset-import flow in T101"
(`tasks/DECISIONS-2026-10-16.md:152`) and a T099 angle
(`docs/IMPLEMENTATION_PLAN.md:299`). Today nothing imports: `AssetsLock` only
records caller-supplied hashes and settings (`crates/crpg-data/src/package.rs:92`),
there is no `import`/`glTF`/`portrait`/`audio` function in `crates/*/src`, and
the bridge has no asset call (`crates/crpg-godot/src/core/mod.rs:99`). This
section designs the flow the M6 `Assets` FFI object (import/scan/thumbnail,
`docs/CRPG_ENGINE_SPEC.md:1025`) and T047 implement.

**The flow, as an author experiences it.**

1. **Drop.** The author drops source files into `assets/models/`,
   `assets/textures/`, `assets/audio/`, `assets/fonts/`
   (`docs/CRPG_ENGINE_SPEC.md:503`), or uses the editor's "Import…" dialog,
   which copies them there.
2. **Scan.** The editor (or `crpgc` import command) scans the folder and, for
   each new or changed source path, computes its BLAKE3 hash and records it
   with default import settings in `assets.lock` (`AssetRecord { hash, import }`,
   `crates/crpg-data/src/package.rs:82`), so imports are reproducible
   (`docs/CRPG_ENGINE_SPEC.md:518`).
3. **Conformance as a gate (R15).** Each asset is validated before it is
   usable: glTF must carry a `SkeletonProfileHumanoid` rig with no Transform
   or Scale on skeleton or parent nodes (`docs/research/T099/R15-deep.md:5,6`);
   names and ids are capped at 16 characters (`docs/research/T099/R15-deep.md:8`);
   portrait dimensions and texture map sets are checked
   (`docs/research/T099/R15-deep.md:11`). A failure is a diagnostic with a
   pointer into the source path, and the asset is **not** added to
   `assets.lock` until it passes — validation gates packaging with a non-zero
   exit (`docs/research/T099/R15-deep.md:13`).
4. **Thumbnail.** The `Assets` object generates a preview thumbnail for the
   asset browser, keyed by the same hash, so the explorer can show it without
   opening the source.
5. **Conflict check (R15).** Before the asset is referenced or packaged,
   duplicate ids, filenames, rows and base-game overrides are reported
   (`docs/research/T099/R15-deep.md:9`); the packaging dialog refuses to
   package with unresolved conflicts or errors (`docs/CRPG_ENGINE_SPEC.md:1075`).
6. **Lock and digest.** `campaign.lock` records the one `assets.lock` digest
   (`assets_lock_digest`, `crates/crpg-data/src/package.rs:241`), keeping
   package versions/checksums and the source-asset digest separate
   (`docs/architecture/crpg-data.md:37`). A content-hashed manifest is the
   packaged-byte authority (`docs/research/T099/R15-deep.md:10`); the
   packaged `manifest.json` writer is T093 (`docs/IMPLEMENTATION_PLAN.md:393`).

**Principles this flow enforces.**

- **The library does not read the source asset** (`crates/crpg-data/src/package.rs:82`):
  hashing and conformance live in the importer, not in `crpg-data`; the data
  crate stays I/O-free and only records what the importer reports.
- **Import is idempotent and reversible.** Re-running import re-hashes and
  no-ops on unchanged paths; rejecting a transformed skeleton is a warning
  that becomes a blocking error, and the `Overwrite Axis` option is previewed
  and reversible (`docs/research/T099/R15-deep.md:7`).
- **The agent uses the same flow headlessly.** `crpgc import` (or `apply` with
  an asset verb) performs the identical scan-and-conformance steps, so an
  agent's asset addition passes the same gate as a human's, with the same
  diagnostics (the "Agent-generated content cannot bypass invariants the GUI
  enforces" rule, `docs/CRPG_ENGINE_SPEC.md:1013`).
- **glTF 2.0 in, Godot's pipeline, content-hashed** is the pinned format
  decision (`docs/CRPG_ENGINE_SPEC.md:1750`); the flow does not add a second
  pipeline.

## Usability acceptance criteria per M6 package

"Every M6 contract carries the usability criteria T101 sets for it"
(`docs/IMPLEMENTATION_PLAN.md:307`). Each criterion is measurable and names
how to check it. The methods are R11's: tiny think-aloud rounds of ~5 testers
(`docs/research/T099/R11-deep.md:5`), 3–5 evaluator heuristic passes
(`docs/research/T099/R11-deep.md:7`), cognitive walkthroughs for first-run and
novel flows (`docs/research/T099/R11-deep.md:9`), workflows over features
(`docs/research/T099/R11-deep.md:19`), and a quantitative release gate
(`docs/research/T099/R11-deep.md:17`). The user is the first human tester,
then recruits module authors (spec risk 18, `docs/CRPG_ENGINE_SPEC.md:1785`);
recruitment is opt-in with informed consent (`docs/research/T099/R11-deep.md:13`).

- **T058 (`crpg-edit`).** *Criterion:* an agent, given only `crpgc` and the
  schemas, assembles a §17.2-shaped campaign through `apply` with every error
  actionable (pointer + code + suggested fix) and no unrecoverable refusal.
  *Check:* the T100 benchmark baseline (`docs/IMPLEMENTATION_PLAN.md:300`),
  scored on validity, playability, steps and errors.
- **T059 (`crpgc apply`).** *Criterion:* a person can read an `apply` plan and
  predict exactly which objects change, and an agent can write one from the
  verb descriptions alone (no examples). *Check:* cognitive walkthrough of a
  printed plan; T100 re-run against the JSON format.
- **T060 (editor shell: explorer + problems).** *Criterion:* a new author
  finds any campaign object in ≤ 3 clicks, and every Problems-panel entry is
  click-to-jump to its source. *Check:* think-aloud; count clicks and jumps.
- **T061 (schema-generated property forms).** *Criterion:* adding a field to a
  schema shows in the form with zero UI work, and an author completes a
  creature form without knowing the schema exists. *Check:* regenerate a field
  and confirm the form renders it; think-aloud on the creature form.
- **T062 (placement tools).** *Criterion:* select/move/rotate/snap/duplicate/
  delete are all undoable and discoverable; an author places three props and
  two creatures in ≤ 2 minutes without the docs. *Check:* think-aloud with a
  timed placement brief.
- **T063 (terrain).** *Criterion:* an author sculpts a simple heightfield in
  ≤ 3 brush operations, undo restores it byte-identically, no crash. *Check:*
  think-aloud plus a scripted undo probe.
- **T064 (navmesh bake + overlay).** *Criterion:* an author bakes and sees the
  walkable overlay in ≤ 2 actions, and a walled-off region is visibly
  non-walkable. *Check:* cognitive walkthrough.
- **T065 (creature editor).** *Criterion:* an author creates a creature, sets
  `minimal-d6`, and sees the full stat block and modifier breakdown without
  ever seeing JSON. *Check:* think-aloud (the same task the H-author gate
  uses, `docs/IMPLEMENTATION_PLAN.md:310`).
- **T066 (dialogue editor + text round trip).** *Criterion:* an author builds
  the §17.2 three-node dialogue in graph view, toggles to text and back with
  no change, and walks it in the simulator with all nodes reachable. *Check:*
  think-aloud plus a byte-identical round-trip test (`docs/CRPG_ENGINE_SPEC.md:663`).
- **T067 (quest editor).** *Criterion:* an author builds the two-state quest
  and the reachability analysis confirms `completed` is reachable from the
  start, flagged live. *Check:* cognitive walkthrough.
- **T068 (Play).** *Criterion:* an author presses Play and reaches the combat
  log with ≤ 1 request for help; save/quit/load reproduces identical state
  (§17.2 item 11). *Check:* think-aloud across §17.2 items 6–11; the
  save/load state-identity probe.
- **T103 (intent layer, M6-R).** *Criterion:* the GUI's form edits and an
  agent's `apply` verbs produce byte-identical receipts for the same edit.
  *Check:* a scripted GUI edit replayed as `apply`, asserting equal
  `CommandReceipt` (`crates/crpg-edit/src/document.rs:75`).
- **T110 (ruleset from template, post-M6).** *Criterion:* an author clones
  `srd-lite`, patches one ability and one outcome table, and the derived-stat
  expressions validate headlessly, without writing a full ruleset by hand.
  *Check:* think-aloud plus a ruleset dry-run (`docs/research/T099/R13-deep.md:17`).

**Cross-cutting gate.** The M6 exit is the H-author: the §17.2 campaign
"created entirely in the editor by someone who has never seen the JSON"
(`docs/CRPG_ENGINE_SPEC.md:1606`). Every package's criterion is a slice of
that one end-to-end proof, so no package is accepted on its own criterion
alone; Greenhollow (`docs/CRPG_ENGINE_SPEC.md:1537`) is the first honest
editor-usability test after it.

## Decisions

Each decision is numbered and states the alternative it rejects.

1. **Personas: module authors first, agents equal, own game last.** Fixed by
   D28l (`tasks/DECISIONS-2026-10-16.md:135`) and adopted unchanged. *Rejected:*
   optimising first for the studio's own game, or demoting agents to a
   second-class, best-effort surface. Both would have made the headless-first
   order (D18) a bolt-on instead of the spine.
2. **An intent layer above `EditCommand`, not instead of it.** D28j fixes the
   position (`tasks/DECISIONS-2026-10-16.md:105`). *Rejected:* (a) exposing raw
   `EditCommand` + JSON pointers to agents — R4 shows identity must be the
   address and raw pointers are too low-level to be the author's model; (b)
   replacing `EditCommand` with the verbs — that discards the closed-set,
   atomic, property-tested core that T058 just built.
3. **Scaffold, never blank.** Every `create` yields a structurally valid
   document of the current schema tag (R8, `docs/research/T099/R8-deep.md:9`).
   *Rejected:* a blank template the author fills, which leaves the campaign
   invalid between keystrokes and teaches bad states.
4. **Identity is the address; JSON Pointers only for leaves.** Objects are
   named by `Ulid`, fields by schema name (R4,
   `docs/research/T099/R10-deep.md:27`). *Rejected:* raw JSON Pointers as the
   author-facing address, which break on every reordering and expose the
   serialization instead of the model.
5. **Property forms generated from schemas.** The schema the data crate
   already emits is the form generator (`docs/architecture/crpg-data.md:19`).
   *Rejected:* hand-written forms, which "are how editors rot"
   (`docs/CRPG_ENGINE_SPEC.md:1059`).
6. **Problems panel first.** Continuous validation on every change is the
   first thing polished, ahead of visual polish (R8,
   `docs/research/T099/R8-deep.md:7`). *Rejected:* shipping the 3D viewport
   first and validating later, which is the Aurora-Toolset trap the spec names
   (`docs/CRPG_ENGINE_SPEC.md:1057`).
7. **Dialogue: graph primary, text a lossless round trip.** Node graph is the
   primary view; indented text round-trips losslessly with JSON as the source
   of truth (`docs/CRPG_ENGINE_SPEC.md:663`; R9,
   `docs/research/T099/R9-deep.md:5`). *Rejected:* graph-only (agents and
   writers want text) and text-only (authors want to see the graph).
8. **References panel on every object.** "What uses this / what does this use"
   makes deletion safe (`docs/CRPG_ENGINE_SPEC.md:1058`). *Rejected:* silent
   deletion with a later dangling-reference surprise.
9. **Ruleset from template with patch overrides, not copy-only.** The rules
   editor clones a starter and stores overrides as a diff against the base
   (R13, `docs/research/T099/R13-deep.md:9`; `rules/overrides.json`,
   `docs/CRPG_ENGINE_SPEC.md:1070`). *Rejected:* copy-only creation, which
   forks every ruleset and breaks the base's fixes; and blank rulesets, which
   violate "scaffold, never blank".
10. **Asset import gates on conformance.** Rig standard, no transformed
    skeletons, 16-char ids, conflict check, and validation gates packaging
    (R15, `docs/research/T099/R15-deep.md:5-9,13`). *Rejected:* accept-anything
    with warnings, which moves failure to the player's client and breaks
    shared modules.
11. **Headless command API first, GUI second.** D18 and spec Phase 6 risk
    ("no editor ships without a corresponding headless command API",
    `docs/CRPG_ENGINE_SPEC.md:1607`). *Rejected:* GUI-first, which strands
    agents and the T100 benchmark.
12. **One editor in `crpg-godot`, with none of Godot's vocabulary.** The
    `editor/` shell is a `crpg-godot` surface (T060), but the UI names are the
    spec's (`docs/CRPG_ENGINE_SPEC.md:1031`). *Rejected:* a separate
    non-Godot editor stack, which would fork the bridge and the asset pipeline.

## Consequences

- **T059 and T103 reorder relative to the current plan.** T059's `crpgc apply`
  JSON "follows the T101 ADR" (`docs/IMPLEMENTATION_PLAN.md:316`), so its
  contract cannot be written until the intent-layer verb list (§ Intent
  command layer) is accepted. T103 (the intent layer, in `crpg-edit`) must
  land before or with T059, not after. The plan lists T103 as "if T101 keeps
  it" (`docs/IMPLEMENTATION_PLAN.md:303`); this draft keeps it, so the manager
  promotes T103 to a prerequisite of T059.
- **T053's contract waits on this draft.** "its contract waits on the T101
  draft's event-authoring section" (`docs/IMPLEMENTATION_PLAN.md:283`). This
  draft's § Task flows covers the MVP quest hook, but it does **not** design
  the visual event-graph authoring surface — that is spec §11.4's event graph
  editor (`docs/CRPG_ENGINE_SPEC.md:1071`) and M8's T073/T074
  (`docs/IMPLEMENTATION_PLAN.md:346`), which the MVP excludes
  (`docs/CRPG_ENGINE_SPEC.md:1608`). The architect's final ADR must either
  state that explicitly or add the event-authoring section T053 waits on.
- **The bridge grows an editor surface, not a second authority.** T060–T068
  add `editor/` to `crpg-godot`, but every edit still flows through the
  `crpg-edit` document and intent layer. The bridge's current open inputs
  (campaign dir, encounter, side, seed — `crates/crpg-godot/src/core/mod.rs:99`)
  are unchanged for the runtime path; the editor path is additive.
- **No new crate is required for the intent layer.** It is a module of
  `crpg-edit` (T103's owner), which keeps the closed command set, the caps and
  the receipts in one place. A new crate would fragment the one-implementation
  rule.
- **The `Assets` FFI object and the importer are new surface.** They live in
  the editor/importer (not `crpg-data`, which stays I/O-free per
  `crates/crpg-data/src/package.rs:82`). T047 owns the importer; its contract
  picks up the conformance gate and idempotence this section pins.
- **M6 acceptance changes.** Every M6 contract now carries a usability
  criterion (§ Usability acceptance criteria), and M6-R's exit gate — "No M6
  contract is approved before then" (`tasks/DECISIONS-2026-10-16.md:108`) —
  now has content to check against. The T102 prototype walkthrough
  (`docs/IMPLEMENTATION_PLAN.md:302`) validates these flows before any M6 code
  is committed.

## Open questions (each with a recommendation)

1. **Is the intent layer a module of `crpg-edit` or a new crate?**
   *Recommendation:* a module of `crpg-edit`, since T103 names it as the owner
   (`docs/IMPLEMENTATION_PLAN.md:303`) and a separate crate would fragment the
   one-implementation rule.
2. **Does `create` reuse `crpgc new`'s scaffolding, or does the editor need
   its own scaffold path?** `crpgc new` writes one typed document to stdout
   (`docs/architecture/crpg-cli.md:610`). *Recommendation:* `create` reuses the
   same document-building path but installs it via `CreateDocument`; the
   scaffold is one code path, two entry points.
3. **Is `link` a first-class verb or a `set` of an id string?** *Recommendation:*
   keep `link` first-class (Decision 4 and § Intent command layer), because a
   dangling reference is a receipt diagnostic, not an error, and that
   distinction is worth a verb of its own.
4. **Where do ruleset templates live, and how do overrides serialize?**
   *Recommendation:* starters ship as the existing `rulesets/minimal-d6` and
   `rulesets/srd-lite` (`rulesets/minimal-d6/ruleset.json:21`,
   `rulesets/srd-lite/ruleset.json:31`); overrides are a diff stored in
   `rules/overrides.json` (`docs/CRPG_ENGINE_SPEC.md:1070`), with the exact
   diff schema left to T110.
5. **Which asset formats in the MVP?** *Recommendation:* glTF models and
   portraits first, audio and fonts deferred (MVP excludes music and the
   broader pipeline, `docs/CRPG_ENGINE_SPEC.md:1531`); T105 decides the
   specific rig packs before T047 (`docs/IMPLEMENTATION_PLAN.md:275`).
6. **Are the intent layer's actionable errors a new public enum or serialized
   triples?** *Recommendation:* a new error type in `crpg-edit` distinct from
   `EditError`, serialized as `{ pointer, code, suggestion }` for `crpgc apply`,
   because `EditError`'s 16 variants are pinned and must not grow for the
   layer's did-you-mean diagnostics (`crates/crpg-edit/src/error.rs:28`).
7. **Is the stable-address field a schema field name or a slug?**
   *Recommendation:* schema field names; slugs are document identity, not
   addresses, and the layer must not conflate them.
8. **Do the runtime §17.2 items (6–12) need authoring flows or only
   verification?** *Recommendation:* only the prerequisite-authoring flows as
   written in § Task flows, with the runtime proof left to M7's acceptance
   (`docs/IMPLEMENTATION_PLAN.md:336`).
9. **Does this draft cover the event-authoring section T053 waits on?**
   It does not; the MVP's trigger is "one line of Lua or a hardcoded quest
   hook" (`docs/CRPG_ENGINE_SPEC.md:1531`) and the visual event graph is
   excluded from Phase 6 (`docs/CRPG_ENGINE_SPEC.md:1608`). *Recommendation:*
   the final ADR states that deferral explicitly so T053's dependency is
   discharged, or adds the event-authoring section before acceptance.
10. **Do the read-only commands (`explain`, `schema`, `validate`) join the
    intent layer or stay in `crpgc`?** *Recommendation:* they stay as-is in
    `crpgc` (`crates/crpg-cli/src/main.rs:110`); the layer adds only
    mutations plus `plan`/`apply_plan`/`validate`, so the verb count stays
    under twenty (R7, `docs/research/T099/R10-deep.md:42`).

## Agent log

- 2026-11-19 (UTC) · opencode-go/deepseek-v4-pro + T101 draft b · Filed the blind authoring UX ADR draft: module authors first with agents equal (D28l), a scaffold-never-blank intent layer of fourteen verbs above T058's EditCommand shared by the GUI and crpgc apply, thirteen task flows covering every §17.2 item and ruleset-from-template, a three-region editor with generated forms and a continuous Problems panel, a conformance-gated asset-import flow, and measurable usability acceptance criteria per M6 package, each with a rejected alternative and unresolved items left as open questions.
