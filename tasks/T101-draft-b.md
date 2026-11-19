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

## Information architecture

## Intent command layer

## Asset import

## Usability acceptance criteria per M6 package

## Decisions

## Consequences

## Open questions (each with a recommendation)

## Agent log
