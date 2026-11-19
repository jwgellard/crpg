# T101 fact sheet — the authoring UX ADR

Facts only. Every citation is `path:line` on `task/T101-facts-f1`.

## Requirements

**Spec — the MVP and what "authoring" must achieve**

- `Spec §17.1 MVP purpose` (`docs/CRPG_ENGINE_SPEC.md:1512`): one area, two creatures, one NPC, one quest, one weapon, one ~12-stat ruleset, proving `Editor → Campaign Data → Server → Client`.
- `Spec §17.2 In scope` (`docs/CRPG_ENGINE_SPEC.md:1514`): the twelve binding MVP items.
- `Spec §17.2 item 1` (`docs/CRPG_ENGINE_SPEC.md:1516`): "Editor creates an area with a flat or simple heightfield terrain and a few placed props."
- `Spec §17.2 item 6` (`docs/CRPG_ENGINE_SPEC.md:1521`): "Press Play: editor spawns an in-process server, connects a client."
- `Spec §17.2 item 9` (`docs/CRPG_ENGINE_SPEC.md:1524`): attack, turn-based, real dice, combat log, enemy dies.
- `Spec §17.2 item 12` (`docs/CRPG_ENGINE_SPEC.md:1527`): the same campaign against a separate `crpg-server` with two clients.
- `Spec §17.3 out of scope` (`docs/CRPG_ENGINE_SPEC.md:1531`): no visual event graph editor, no modding UI, no packaging UI in the MVP.
- `Spec §17.4 Greenhollow` (`docs/CRPG_ENGINE_SPEC.md:1537`): "the first honest test of whether the editor is usable."
- `Spec §11.2 sketch` (`docs/CRPG_ENGINE_SPEC.md:1002`): `CampaignDocument::apply/undo/redo/validate/save`.
- `Spec §11.2 undo` (`docs/CRPG_ENGINE_SPEC.md:1012`): "Undo/redo is one implementation", every mutation goes through `apply`.
- `Spec §11.2 agents` (`docs/CRPG_ENGINE_SPEC.md:1013`): agents drive the same API headlessly; "Agent-generated content cannot bypass invariants the GUI enforces."
- `Spec §11.3 FFI objects` (`docs/CRPG_ENGINE_SPEC.md:1025`): roughly five objects — `Document`, `Session`, `Assets` (import/scan/thumbnail), `Diagnostics`, `Preview`.
- `Spec §11.4 main window` (`docs/CRPG_ENGINE_SPEC.md:1031`): three IDE-like regions and none of Godot's vocabulary.
- `Spec §11.4 Problems panel` (`docs/CRPG_ENGINE_SPEC.md:1057`): continuous validation on every change, like a compiler.
- `Spec §11.4 References panel` (`docs/CRPG_ENGINE_SPEC.md:1058`): "What uses this?" and "What does this use?" on every object.
- `Spec §11.4 generated forms` (`docs/CRPG_ENGINE_SPEC.md:1059`): "Property forms are generated from schemas."
- `Spec §11.4 dialogue editor` (`docs/CRPG_ENGINE_SPEC.md:1067`): node graph primary, text toggle, "walk this conversation" simulator, unreachable nodes greyed live.
- `Spec §11.4 rules editor` (`docs/CRPG_ENGINE_SPEC.md:1070`): override a ruleset per campaign; the 2026-10-16 D28l note requires creating a ruleset from a template after Phase 6 (T110).
- `Spec §11.4 event graph editor` (`docs/CRPG_ENGINE_SPEC.md:1071`): node graph over §5.2's IR with generated property forms.
- `Spec §11.4 campaign debugger` (`docs/CRPG_ENGINE_SPEC.md:1073`): entity inspector, variable watch, quest state, effects, modifier breakdown, rules trace, time controls.
- `Spec §11.4 packaging` (`docs/CRPG_ENGINE_SPEC.md:1075`): one dialog; "Refuse to package with unresolved errors".
- `Spec §5.3 dialogue` (`docs/CRPG_ENGINE_SPEC.md:639`): dialogue is a specialised graph, not the general one.
- `Spec §5.3 text round trip` (`docs/CRPG_ENGINE_SPEC.md:663`): "Lossless in both directions"; JSON is the source of truth.
- `Spec §4.8 agent dialogue` (`docs/CRPG_ENGINE_SPEC.md:588`): agents author dialogue in indented text; `crpgc dialogue import/export` converts losslessly.
- `Spec Phase 6 objective` (`docs/CRPG_ENGINE_SPEC.md:1604`): "create the MVP campaign without hand-writing JSON."
- `Spec Phase 6 deliverables` (`docs/CRPG_ENGINE_SPEC.md:1605`): `crpg-edit` document/command/undo/validate; shell, explorer, generated forms, area, creature, dialogue, quest, problems, Play.
- `Spec Phase 6 definition of done` (`docs/CRPG_ENGINE_SPEC.md:1606`): the §17.2 campaign "created entirely in the editor by someone who has never seen the JSON."
- `Spec Phase 6 risk` (`docs/CRPG_ENGINE_SPEC.md:1607`): "no editor ships without a corresponding headless command API."
- `Spec Phase 6 do-not-build` (`docs/CRPG_ENGINE_SPEC.md:1608`): visual event graph, cinematics, the rules editor, packaging UI.
- `Spec §19.1 #24` (`docs/CRPG_ENGINE_SPEC.md:1676`): read-only campaign explorer tree.
- `Spec §19.1 #26` (`docs/CRPG_ENGINE_SPEC.md:1678`): problems panel rendering `Vec<Diagnostic>`.
- `Spec §19.2 #15` (`docs/CRPG_ENGINE_SPEC.md:1697`): document/command/undo with the random-command-sequence property test.
- `Spec §19.2 #16` (`docs/CRPG_ENGINE_SPEC.md:1698`): placement tooling — select, move, rotate, snap, duplicate, delete.
- `Spec §19.2 #19` (`docs/CRPG_ENGINE_SPEC.md:1701`): event IR interpreter with serializable `Wait` continuations.
- `Spec risk 18` (`docs/CRPG_ENGINE_SPEC.md:1785`): "Talk to NWN and Solasta module authors before designing the dialogue and quest editors."
- `Spec Aurora bar` (`docs/CRPG_ENGINE_SPEC.md:73`): the product people "compare favourably to the Aurora Toolset."

**Implementation plan — M6-R and M6**

- `M6-R heading` (`docs/IMPLEMENTATION_PLAN.md:288`): "Authoring UX research (D28j; runs now, in parallel with M4)".
- `M6-R why` (`docs/IMPLEMENTATION_PLAN.md:290`): "Both agents and humans must find campaign authoring easy."
- `M6-R exit` (`docs/IMPLEMENTATION_PLAN.md:293`): T101 accepted and the T102 prototype walked; "No M6 contract is approved before then."
- `T099` (`docs/IMPLEMENTATION_PLAN.md:299`): the eight research angles, one report each.
- `T100` (`docs/IMPLEMENTATION_PLAN.md:300`): agent authoring benchmark scored on validity, playability, steps and errors, including a "define a small rule system" brief.
- `T101` (`docs/IMPLEMENTATION_PLAN.md:301`): the ADR's required contents — personas (module authors first, agents equal: D28l), task flows for each §17.2 item and ruleset-from-template, information architecture, an intent layer above T058, the asset-import flow, usability acceptance criteria per M6 package.
- `T102` (`docs/IMPLEMENTATION_PLAN.md:302`): clickable HTML prototype of the main window and MVP flows.
- `T103` (`docs/IMPLEMENTATION_PLAN.md:303`): the intent-level command layer "if T101 keeps it."
- `M6 heading` (`docs/IMPLEMENTATION_PLAN.md:305`): "Editor v1 (spec Phase 6; headless command API first, per D18)".
- `M6 usability` (`docs/IMPLEMENTATION_PLAN.md:307`): "Every M6 contract carries the usability criteria T101 sets for it."
- `M6 exit` (`docs/IMPLEMENTATION_PLAN.md:310`): H-author builds the MVP campaign in the editor.
- `T058` (`docs/IMPLEMENTATION_PLAN.md:315`): `EditCommand`, document, undo, validation, receipts.
- `T059` (`docs/IMPLEMENTATION_PLAN.md:316`): `crpgc apply`; "its JSON format follows the T101 ADR".
- `T060` (`docs/IMPLEMENTATION_PLAN.md:317`): `editor/` shell, read-only explorer, problems panel.
- `T061` (`docs/IMPLEMENTATION_PLAN.md:318`): schema-generated property forms.
- `T062` (`docs/IMPLEMENTATION_PLAN.md:319`): placement tools.
- `T063` (`docs/IMPLEMENTATION_PLAN.md:320`): terrain heightfield data, then sculpt/paint.
- `T064` (`docs/IMPLEMENTATION_PLAN.md:321`): navmesh bake with a walkable overlay.
- `T065` (`docs/IMPLEMENTATION_PLAN.md:322`): creature editor.
- `T066` (`docs/IMPLEMENTATION_PLAN.md:323`): dialogue editor with text round-trip.
- `T067` (`docs/IMPLEMENTATION_PLAN.md:324`): quest editor.
- `T068` (`docs/IMPLEMENTATION_PLAN.md:325`): Play — in-process server plus client from the editor.
- `T110` (`docs/IMPLEMENTATION_PLAN.md:353`): rules editor after M6, "create a ruleset from a template as well as override one per campaign."
- `T053` (`docs/IMPLEMENTATION_PLAN.md:283`): its contract "waits on the T101 draft's event-authoring section."
- `T104` (`docs/IMPLEMENTATION_PLAN.md:274`): player UX research, before the T050/T051 contracts.
- `T105` (`docs/IMPLEMENTATION_PLAN.md:275`): asset sourcing per spec risk 15, before T047.
- `Wave 4 gate` (`docs/IMPLEMENTATION_PLAN.md:416`): accept the movement ADR, accept the T101 authoring UX ADR, H-proto.

**Decisions**

- `D28j` (`tasks/DECISIONS-2026-10-16.md:90`): the authoring UX research stage before editor contracts.
- `D28j quote` (`tasks/DECISIONS-2026-10-16.md:92`): "a major, major concern is both agent and human usability."
- `D28j intent layer` (`tasks/DECISIONS-2026-10-16.md:105`): a layer above the merged T058 `EditCommand` "is added above it, not in its place."
- `D28j gate` (`tasks/DECISIONS-2026-10-16.md:108`): "No M6 contract is approved before the T101 ADR is accepted."
- `D28l` (`tasks/DECISIONS-2026-10-16.md:125`): audience, rule-defining as a pillar, both time modes.
- `D28l quote` (`tasks/DECISIONS-2026-10-16.md:127`): "Module authors is the primary goal, yes with agents being equal."
- `D28l personas` (`tasks/DECISIONS-2026-10-16.md:135`): "module authors first, agents equal. Your own game comes after them."
- `D28l rulesets` (`tasks/DECISIONS-2026-10-16.md:136`): "Defining a ruleset is an authoring task in its own right"; T101 gains the ruleset flows.
- `D28l asset flow` (`tasks/DECISIONS-2026-10-16.md:152`): "Custom content from module authors: a T099 angle, and the asset-import flow in T101."
- `D29c` (`tasks/DECISIONS-D29-DRAFT.md:32`): "who comes first: primary authors and players"; line 34 records the answer as D28l.
- `D65i` (`tasks/DECISIONS-D65.md:26`): the engine needs "an AI authoring surface", open before the T080/T081 contracts.

## Research findings

Each report below is folded into `docs/research/T099/`; the check results are in `docs/research/T099/README.md:5`.

**R8 — module-editor prior art** (`docs/research/T099/R8-deep.md:1`)

- `Interactive help beats docs` (`docs/research/T099/R8-deep.md:5`): ship a guided walkthrough for the first module.
- `Declarative defaults` (`docs/research/T099/R8-deep.md:6`): keep the default quest/NPC loop declarative; Lua for advanced scripts only.
- `Problems panel first` (`docs/research/T099/R8-deep.md:7`): continuous Problems panel and headless `validate` before visual polish.
- `Crash-safe saves` (`docs/research/T099/R8-deep.md:8`): auto-save, recovery, external backups.
- `Scaffold, never blank` (`docs/research/T099/R8-deep.md:9`): every object starts as a structurally valid document.
- `Diffable JSON` (`docs/research/T099/R8-deep.md:10`): campaign data stays canonical JSON rather than a binary module file. `[check: unavailable — V8]`
- `Templates` (`docs/research/T099/R8-deep.md:11`): support inheritance/overrides, not copy-only creation.
- `One help facility` (`docs/research/T099/R8-deep.md:12`): one prompted help facility, first loop under one session.

**R9 — narrative tools** (`docs/research/T099/R9-deep.md:1`)

- `Graph primary, text lossless` (`docs/research/T099/R9-deep.md:5`): node graph primary; indented text a lossless round trip.
- `Seen/visited operands` (`docs/research/T099/R9-deep.md:12`): make visit counts first-class conditions and quest operands.
- `Real conversation debugger` (`docs/research/T099/R9-deep.md:19`): jump-to-node, live variables, force-assign, undo the last choice.
- `Continuous structural checks` (`docs/research/T099/R9-deep.md:27`): orphan nodes, undefined variables, loose ends, unreachable states.
- `Per-language status` (`docs/research/T099/R9-deep.md:34`): final/in-progress/review/outdated per text, auto-marked.
- `VO on stable ids` (`docs/research/T099/R9-deep.md:41`): tie voice-over and localisation to stable ids at authoring time.
- `Reachability summaries` (`docs/research/T099/R9-deep.md:48`): path counts and impossible branch combinations.
- `Receipts for agent edits` (`docs/research/T099/R9-deep.md:55`): GUI and agents on one transactional command surface with receipts.

**R10 — authoring interfaces for AI agents** (`docs/research/T099/R10-deep.md:1`)

- `R1 intent layer` (`docs/research/T099/R10-deep.md:12`): a few consolidated verbs above `EditCommand`, shared by GUI and agents.
- `R2 actionable errors` (`docs/research/T099/R10-deep.md:17`): machine-readable pointer + code + suggested fix at validate and apply time.
- `R3 plan/apply` (`docs/research/T099/R10-deep.md:22`): a reviewable, saveable plan step; never imply rollback.
- `R4 stable addresses` (`docs/research/T099/R10-deep.md:27`): identity is the agent-visible address; JSON Pointers only for leaves.
- `R5 anchored edits` (`docs/research/T099/R10-deep.md:32`): prefer uniquely-anchored local edits; fail on ambiguous anchors.
- `R6 bounded responses` (`docs/research/T099/R10-deep.md:37`): receipts carry changed paths and diagnostics only.
- `R7 tool descriptions` (`docs/research/T099/R10-deep.md:42`): 3–4 sentences per verb, under ~20 verbs, examples eval-gated.
- `R10 eval harness` (`docs/research/T099/R10-deep.md:57`): end-state scoring, ~20 tasks, held-out set.

**R11 — usability methods** (`docs/research/T099/R11-deep.md:1`)

- `Tiny think-aloud rounds` (`docs/research/T099/R11-deep.md:5`): ~5 testers from the prototype on.
- `Heuristic evaluation` (`docs/research/T099/R11-deep.md:7`): 3–5 evaluators before each user test.
- `Cognitive walkthrough` (`docs/research/T099/R11-deep.md:9`): for the first-run and novel flows.
- `Empty states as onboarding` (`docs/research/T099/R11-deep.md:11`): starter content instead of a blank campaign. `[check: not found — V10]`
- `Volunteer consent` (`docs/research/T099/R11-deep.md:13`): recruit modders with informed consent, opt-in only.
- `Agents as screeners only` (`docs/research/T099/R11-deep.md:15`): LLM "users" are biased proxies, never proof.
- `Quantitative release gate` (`docs/research/T099/R11-deep.md:17`): real tasks and metrics before release.
- `Workflows over features` (`docs/research/T099/R11-deep.md:19`): the prototype walkthrough follows workflows.

**R12 — event and trigger authoring** (`docs/research/T099/R12-deep.md:1`)

- `Canned primitives first` (`docs/research/T099/R12-deep.md:5`): parameterised quest/dialogue/trigger primitives and wizards before any graph.
- `Declarative, frame-atomic IR` (`docs/research/T099/R12-deep.md:11`): bounded `Wait`, no polling; the Papyrus thread limit is marked at line 14 `[check: not found — V8]`.
- `Canonical text form` (`docs/research/T099/R12-deep.md:17`): stable ids and a specified evaluation order; the graph is a view.
- `Budgets` (`docs/research/T099/R12-deep.md:23`): time-slice budgets, no tick-based logic on event paths; line 25's cite is `[check: not found — V8]`.
- `State-driven event pages` (`docs/research/T099/R12-deep.md:29`): conditional pages cover many trigger cases without graphs.
- `Layered stack` (`docs/research/T099/R12-deep.md:35`): wizard actions → text IR → sandboxed Lua, one validation pipeline.
- `Machine-readable diagnostics` (`docs/research/T099/R12-deep.md:41`): rule highlighting, references, pointable errors; line 43 is `[check: not found — V8]`.

**R13 — ruleset authoring** (`docs/research/T099/R13-deep.md:1`)

- `Typed data models, generated UI` (`docs/research/T099/R13-deep.md:5`): inheritable data models with small script hooks; sheets generated from schema.
- `Clone-and-modify starters` (`docs/research/T099/R13-deep.md:7`): ship `minimal-d6`, `srd-lite` and richer examples rather than blank rulesets.
- `Layering and patch overrides` (`docs/research/T099/R13-deep.md:9`): inherit a ruleset and patch one fact by identity.
- `Engine limits as data` (`docs/research/T099/R13-deep.md:11`): validate capacity in schema instead of fixed slot caps.
- `Expression DSL` (`docs/research/T099/R13-deep.md:13`): derived stats through named, validated expressions.
- `Deterministic migration` (`docs/research/T099/R13-deep.md:15`): migrate authored object versions and keep patch ops idempotent.
- `Testing in the editor` (`docs/research/T099/R13-deep.md:17`): headless dry-runs and pack validation, not external tools. `[check: unavailable — V8]`
- `Data, not code access` (`docs/research/T099/R13-deep.md:19`): give modders a data/tool surface, keep arbitrary code privileged.

**R14 — pause and time modes** (`docs/research/T099/R14-deep.md:1`)

- `Host-granted pause` (`docs/research/T099/R14-deep.md:8`): pause is a revocable permission; anyone holding it pauses everyone.
- `Per-encounter switching` (`docs/research/T099/R14-deep.md:9`): RTwP-to-TB on the fly on one `Timeline`, never locked per campaign.
- `Late-join initiative` (`docs/research/T099/R14-deep.md:10`): round 1 keeps a full turn; later joiners skip; tied same-player characters act together.
- `Auto-pause as policy` (`docs/research/T099/R14-deep.md:11`): server-side, per-trigger toggles, host-controlled, default off in multiplayer.
- `Turn timers` (`docs/research/T099/R14-deep.md:12`): delay/skip and a visible order; party AI off in TB.
- `Split parties stay split` (`docs/research/T099/R14-deep.md:13`): only the engaged group enters TB. `[check: unavailable — V9]`
- `Dialogue pause` (`docs/research/T099/R14-deep.md:14`): trivial dialogue and stores do not pause; important dialogue gets a vote with host tiebreak.
- `Fixed 20 Hz step` (`docs/research/T099/R14-deep.md:17`): TB is the same `Timeline` advanced on `EndTurn`; never a variable step.

**R15 — custom content** (`docs/research/T099/R15-deep.md:1`)

- `Rig conformance as a gate` (`docs/research/T099/R15-deep.md:5`): enforce `SkeletonProfileHumanoid`; bone-map warnings become blocking errors.
- `Reject transformed skeletons` (`docs/research/T099/R15-deep.md:6`): no Transform/Scale on skeleton or parent nodes.
- `Opt-in axis overwrite` (`docs/research/T099/R15-deep.md:7`): keep `Overwrite Axis` previewed and reversible.
- `16-character asset ids` (`docs/research/T099/R15-deep.md:8`): reject longer names at import.
- `Conflict check` (`docs/research/T099/R15-deep.md:9`): duplicate ids, filenames, rows and base-game overrides before packaging.
- `Content-hashed manifests` (`docs/research/T099/R15-deep.md:10`): packs of hashed assets with an explicit control/merge entry.
- `Portrait and texture specs` (`docs/research/T099/R15-deep.md:11`): validate portrait dimensions and generate variants; line 12 requires complete texture map sets.
- `Validation gates packaging` (`docs/research/T099/R15-deep.md:13`): non-zero exit on asset errors, mirroring glTF validation.

## The edit API today

**`crates/crpg-edit/src/` — every public item**

- `EditTarget` (`crates/crpg-edit/src/command.rs:5`): `enum EditTarget { Document(SourcePath), Object(Ulid) }`; an object pointer is relative and may be empty.
- `EditCommand` (`crates/crpg-edit/src/command.rs:15`): "The closed set of editor mutations. Every mutation goes through one of these."
- `CreateDocument` (`crates/crpg-edit/src/command.rs:17`): `{ path: SourcePath, document: Document }`, added at an unused path.
- `DeleteDocument` (`crates/crpg-edit/src/command.rs:24`): `{ path: SourcePath }`.
- `RenameDocument` (`crates/crpg-edit/src/command.rs:29`): `{ from: SourcePath, to: SourcePath }`, bytes unchanged.
- `SetValue` (`crates/crpg-edit/src/command.rs:37`): `{ target: EditTarget, pointer: String, value: Vec<u8> }`, JSON text.
- `InsertValue` (`crates/crpg-edit/src/command.rs:46`): `{ target, pointer, value }`, array index or `-`.
- `RemoveValue` (`crates/crpg-edit/src/command.rs:55`): `{ target, pointer }`.
- `DeleteObject` (`crates/crpg-edit/src/command.rs:63`): `{ id: Ulid }`; a root object deletes its document.
- `PlaceInstance` (`crates/crpg-edit/src/command.rs:69`): `{ area: Ulid, placement: Placement }`.
- `MovePlacement` (`crates/crpg-edit/src/command.rs:76`): `{ placement: Ulid, transform: Transform }`.
- `MAX_BATCH_COMMANDS` (`crates/crpg-edit/src/document.rs:23`): 1,024.
- `MAX_POINTER_BYTES` (`crates/crpg-edit/src/document.rs:25`): 4,096.
- `MAX_VALUE_BYTES` (`crates/crpg-edit/src/document.rs:27`): 1 MiB.
- `MAX_DOCUMENT_BYTES` (`crates/crpg-edit/src/document.rs:29`): 4 MiB.
- `MAX_UNDO_ENTRIES` (`crates/crpg-edit/src/document.rs:31`): 256.
- `MAX_HISTORY_BYTES` (`crates/crpg-edit/src/document.rs:33`): 32 MiB.
- `CampaignDocument` (`crates/crpg-edit/src/document.rs:44`): private fields — loaded campaign, canonical bytes per path, saved baseline, revision, history.
- `ChangeKind` (`crates/crpg-edit/src/document.rs:54`): `Created`, `Modified`, `Deleted`.
- `PathChange` (`crates/crpg-edit/src/document.rs:65`): `{ path: SourcePath, kind: ChangeKind }`.
- `CommandReceipt` (`crates/crpg-edit/src/document.rs:75`): `{ revision: u64, changes: Vec<PathChange>, diagnostics: Vec<Diagnostic> }`; also the view change notification.
- `SavePlan` (`crates/crpg-edit/src/document.rs:87`): `{ revision, write: BTreeMap<SourcePath, Vec<u8>>, remove: BTreeSet<SourcePath> }`.
- `open` (`crates/crpg-edit/src/document.rs:229`): `pub fn open(campaign: LoadedCampaign) -> Result<Self, EditError>`.
- `campaign` (`crates/crpg-edit/src/document.rs:247`): `pub fn campaign(&self) -> &LoadedCampaign`.
- `canonical_files` (`crates/crpg-edit/src/document.rs:252`): `pub fn canonical_files(&self) -> &BTreeMap<SourcePath, Vec<u8>>`.
- `revision` (`crates/crpg-edit/src/document.rs:257`): `pub fn revision(&self) -> u64`.
- `apply` (`crates/crpg-edit/src/document.rs:262`): `pub fn apply(&mut self, command: EditCommand) -> Result<CommandReceipt, EditError>`.
- `apply_batch` (`crates/crpg-edit/src/document.rs:267`): `pub fn apply_batch(&mut self, commands: Vec<EditCommand>) -> Result<CommandReceipt, EditError>`; one undo entry.
- `undo` / `redo` (`crates/crpg-edit/src/document.rs:312`, `crates/crpg-edit/src/document.rs:321`): both return `Result<CommandReceipt, EditError>`.
- `undo_depth` / `redo_depth` / `history_bytes` (`crates/crpg-edit/src/document.rs:330`, `crates/crpg-edit/src/document.rs:335`, `crates/crpg-edit/src/document.rs:340`): `-> usize`.
- `validate` (`crates/crpg-edit/src/document.rs:345`): `pub fn validate(&self) -> Vec<Diagnostic>`.
- `save_plan` (`crates/crpg-edit/src/document.rs:350`): `pub fn save_plan(&self) -> SavePlan`.
- `mark_saved` (`crates/crpg-edit/src/document.rs:371`): `pub fn mark_saved(&mut self, plan: &SavePlan) -> Result<(), EditError>`.
- `EditLimit` (`crates/crpg-edit/src/error.rs:7`): `Pointer`, `Value`, `Document`.
- `EditError` (`crates/crpg-edit/src/error.rs:28`): 16 variants — `Invalid`, `BatchSize`, `InputTooLarge`, `MissingPath`, `PathExists`, `UnknownObject`, `WrongKind`, `Protected`, `Pointer`, `IdentityChanged`, `Rejected`, `HistoryTooLarge`, `NothingToUndo`, `NothingToRedo`, `StaleSavePlan`, `HistoryCorrupt`; pinned lowercase `Display`.
- `root re-exports` (`crates/crpg-edit/src/lib.rs:13`): `EditCommand`, `EditTarget`, the six caps, `CampaignDocument`, `ChangeKind`, `CommandReceipt`, `PathChange`, `SavePlan`, `EditError`, `EditLimit`; `history` is private.

**`docs/architecture/crpg-edit.md` — sections**

- `Scope` (`docs/architecture/crpg-edit.md:3`): the headless document model; GUI, `crpgc apply` and agents share one implementation; D18 orders it before UI.
- `Where it sits` (`docs/architecture/crpg-edit.md:33`): above `crpg-data`; normal deps `crpg-core`, `crpg-data`, `crpg-rules`; `crpg-server` excludes it.
- `Module flow` (`docs/architecture/crpg-edit.md:50`): `command`, `error`, `document`, private `history`; the §5.3 algorithm.
- `Structural failures block` (`docs/architecture/crpg-edit.md:79`): structural refusal versus reported semantic findings; identity is caller-supplied.
- `No migration` (`docs/architecture/crpg-edit.md:97`): `crpg-edit` never migrates; callers refuse pre-migration or `crpgc fmt` drift.
- `Save plan deviation` (`docs/architecture/crpg-edit.md:108`): spec §11.2's `save` is replaced by `save_plan`/`mark_saved`, keeping the library I/O-free.
- `Planned` (`docs/architecture/crpg-edit.md:117`): T059 `crpgc apply` (JSON command form, id generation, preflight, writes), T060–T068 views, D03/T085 live edits, measured optimisation.

## The CLI an agent uses today

`crpgc` has eleven subcommands, parsed by hand-rolled `args_os` per-command parsers, with `0` success / `1` domain failure / `2` usage (`docs/architecture/crpg-cli.md:72`). The dispatch enum is `crates/crpg-cli/src/main.rs:110`.

- `replay` (`crates/crpg-cli/src/main.rs:111`; contract `docs/architecture/crpg-cli.md:116`): `crpgc replay <replay-path> [--golden <path>] [--campaign <dir>]`; silent on success, exit 1 on any typed replay or campaign failure.
- `validate` (`crates/crpg-cli/src/main.rs:116`; contract `docs/architecture/crpg-cli.md:407`): `crpgc validate <campaign-root> [--json]`; plain mode prints one `Display` line per diagnostic to stderr; JSON mode writes one canonical array to stdout.
- `validate --json shape` (`docs/architecture/crpg-cli.md:444`): the complete vector through `crpg_data::canonical_json` — `[]\n` when clean, six-field snake_case diagnostic objects with explicit nulls (`docs/architecture/crpg-data.md:65`).
- `migrate` (`crates/crpg-cli/src/main.rs:120`; contract `docs/architecture/crpg-cli.md:503`): `crpgc migrate <campaign-root>`; the explicit save, silent on success, every flag including `--json` is usage.
- `new` (`crates/crpg-cli/src/main.rs:123`; contract `docs/architecture/crpg-cli.md:610`): `crpgc new <type> --slug <s> --id <id> [--entry-id <id>]`; type is `creature`/`item`/`dialogue`/`quest`, writes one canonical document to stdout.
- `schema` (`crates/crpg-cli/src/main.rs:129`; contract `docs/architecture/crpg-cli.md:611`): `crpgc schema <type>`; writes the generated schema bytes for one of seventeen stems (`docs/architecture/crpg-cli.md:672`).
- `explain` (`crates/crpg-cli/src/main.rs:132`; contract `docs/architecture/crpg-cli.md:612`): `crpgc explain <id> [--root <campaign-root>]`; emits `explain_object`'s canonical report bytes or exits 1 with `object not found`.
- `fmt` (`crates/crpg-cli/src/main.rs:136`; contract `docs/architecture/crpg-cli.md:613`): `crpgc fmt [<campaign-root>] [--check]`; saves canonical bytes or lists noncanonical files on exit 1.
- `lock` (`crates/crpg-cli/src/main.rs:140`; contract `docs/architecture/crpg-cli.md:614`): `crpgc lock [<campaign-root>] --catalog <catalog-path>`; creates, replaces or no-ops `campaign.lock`.
- `run` (`crates/crpg-cli/src/main.rs:144`; contract `docs/architecture/crpg-cli.md:615`): `crpgc run --ticks N --hash-every M [--seed S]`; prints sampled empty-world hashes, a harness rather than campaign execution.
- `demo` (`crates/crpg-cli/src/main.rs:149`; contract `docs/architecture/crpg-cli.md:297`): `crpgc demo <replay-path> --campaign <dir> [--delay-ms N]`; prints a combat replay, a viewer not a second rules path.
- `play` (`crates/crpg-cli/src/main.rs:154`; contract `docs/architecture/crpg-cli.md:344`): `crpgc play --campaign <dir> [--encounter <ulid>] [--side <name>] [--seed N]`; the person chooses from `legal_actions`.
- `--json` coverage (`crates/crpg-cli/src/main.rs:118`): only `validate` parses it; no other subcommand accepts it (`docs/architecture/crpg-cli.md:513`).
- `T013 scaffolding` (`docs/architecture/crpg-cli.md:739`): `new` takes explicit caller-supplied ids and builds typed `Document` variants through `write_document`; no clock, RNG or registry.
- `T013a introspection` (`docs/architecture/crpg-data.md:112`): `explain_object` returns one object plus lexically ordered inbound/outbound reference occurrences as canonical JSON bytes.
- `crpgc apply does not exist` (`crates/crpg-cli/src/main.rs:110`): there is no `apply` variant; T059 owns it and its JSON format waits on T101 (`docs/IMPLEMENTATION_PLAN.md:316`).
- `dialogue import/export does not exist` (`docs/CRPG_ENGINE_SPEC.md:588`): the spec names `crpgc dialogue import/export`; no such subcommand is in the enum.

## Campaign and ruleset data an editor edits

**Top-level campaign format**

- `Campaign` (`crates/crpg-data/src/document.rs:17`): `{ id: Ulid, slug, name, _note?, package: PackageId, version, engine: VersionReq, requires: Vec<PackageRequirement>, entry: EntryPoint }`.
- `EntryPoint` (`crates/crpg-data/src/document.rs:45`): `{ world: Ulid, area: Ulid, spawn: Ulid }` — the starting world, area and placement.
- `Campaign layout` (`docs/CRPG_ENGINE_SPEC.md:480`): `campaign.json`, `campaign.lock`, `schemas/`, `worlds/`, `areas/<area>/`, per-kind directories, `variables/`, `assets/`, `locale/`, `tests/`.
- `campaign.json` (`docs/CRPG_ENGINE_SPEC.md:482`): "manifest: id, name, version, requires[], entry point".
- `campaign.lock` (`docs/CRPG_ENGINE_SPEC.md:485`): resolved package versions/checksums plus the `assets.lock` digest.
- `placements.json` (`docs/CRPG_ENGINE_SPEC.md:496`): every placed instance, separate from `area.json` so the churny file stays separate.
- `assets.lock` (`docs/CRPG_ENGINE_SPEC.md:506`): "path → blake3 hash → import settings".
- `locale/en.json` (`docs/CRPG_ENGINE_SPEC.md:508`): every display text is a key.
- `Canonical writing` (`docs/CRPG_ENGINE_SPEC.md:450`): the editor writes through a canonical writer; `crpgc fmt` enforces it.

**Published schemas**

- `Schema module` (`docs/architecture/crpg-data.md:19`): `schema` "generates 17 self-contained draft-2020-12 schemas from Rust shapes" — the pre-T142 count; later sections say `generated_schemas` is 23 files (`docs/architecture/crpg-data.md:369`).
- `Schema files` (`docs/architecture/crpg-data.md:369`): 23 checked-in files in `schemas/`; the directory holds `campaign`, `world`, `area`, `creature`, `item`, `dialogue`, `quest`, `faction`, `graph`, `placements`, `triggers`, `locale`, `variables`, `campaign-lock`, `assets-lock`, `placement`, `action-signature`, `ruleset`, `ability`, `outcome-table`, `effect`, `encounter`, `ui`.
- `Schema stems` (`docs/architecture/crpg-cli.md:672`): `crpgc schema <type>` accepts seventeen stems; placement and action-signature are embedded roots, not document kinds.
- `Schema drift` (`docs/architecture/crpg-data.md:84`): an explicit `generate_schemas` example writes the baseline; the read-only `schema_drift` test compares exact bytes.

**`crpg.ui/1` (T142)**

- `UI document` (`docs/architecture/crpg-data.md:364`): `crpg.ui/1` is presentation-only, shipped by a ruleset and copied to `ui/<package>.json`.
- `UI identity` (`docs/architecture/crpg-data.md:370`): no `id`, `slug` or `name`; a required `ruleset` field; not in the object index.
- `UI tolerant read` (`docs/architecture/crpg-data.md:376`): six closed sets decode unknown spellings to `Unknown` and re-encode byte-identically; a misspelt kind fails `validate`, not `load_campaign`.
- `UI bindings` (`docs/architecture/crpg-data.md:384`): stats by name, pools by template ULID, abilities/effects by ULID, outcomes by label.
- `UI defaults` (`docs/architecture/crpg-data.md:391`): `generated_ui` and `effective_ui` are pure; an authored entry is never reordered or dropped.
- `UI schema registration` (`crates/crpg-data/src/schema.rs:143`): `document_schema!(UiDocument, "crpg.ui/1", "ui.schema.json")`.
- `UI types` (`crates/crpg-data/src/ui.rs:88`): "One `crpg.ui/1` document: presentation data for one ruleset."

**Ruleset surface**

- `Ruleset` (`docs/architecture/crpg-data.md:146`): stat declarations, health designation, attribute list, pool template, ability references, package self-identity.
- `Real-time fields (T136a)` (`docs/architecture/crpg-data.md:195`): optional `round_ticks`, `recovery_ticks`, `repeats_in_real_time` at `crpg.ruleset/2` and `crpg.ability/2`.
- `Kernel range (T136b)` (`docs/architecture/crpg-data.md:209`): stat kinds `Fixed`, `Bool`, `Enum`, `Tags`, `Dice`; derived stats; defenses; initiative; effect grants.
- `Pools and costs (T017a)` (`docs/architecture/crpg-data.md:171`): plural `Ruleset.pools`, primary-pool `cost` plus `extra_costs`.
- `Preview policy (T143a)` (`docs/architecture/crpg-data.md:480`): optional `Ruleset.preview` with `when` and `max_cost`; absence means `never`.
- `Movement block (T148)` (`docs/architecture/crpg-data.md:430`): optional `Ruleset.movement` (`speed`, `measure`, `turn`) at the current tag, no version bump.
- `Movement types` (`docs/architecture/crpg-data.md:435`): `MovementWire`, `TurnMovementWire`, `LengthWire`, `MeasureWire`; `MAX_SPEED_PER_TICK_RAW` is the one speed bound.
- `Ruleset migration` (`docs/architecture/crpg-data.md:176`): `crpg.ruleset/1 → /2` and `crpg.ability/1 → /2` edges with clone-then-publish defaults.

**Authored fixtures**

- `rulesets/minimal-d6` (`rulesets/minimal-d6/ruleset.json:21`): 4 files, 11 KiB; ruleset at `crpg.ruleset/1` with `action_pool`, plus `strike.json` and `roll_under.json`.
- `rulesets/srd-lite` (`rulesets/srd-lite/ruleset.json:31`): 7 files, 23 KiB; ruleset at `crpg.ruleset/2` with plural `pools`, two abilities, one effect, two outcome tables.
- `campaigns/fixtures/combat_basic` (`campaigns/fixtures/combat_basic/campaign.json:10`): 15 files, 25 KiB; package `fixture.combat-basic`, a copy of the `minimal-d6` content with ability, creature, encounter, table, locale and lock documents.
- `campaigns/fixtures/combat_srd` (`campaigns/fixtures/combat_srd/campaign.json:10`): 18 files, 40 KiB; package `fixture.combat-srd`, a copy of the `srd-lite` content with effects and two outcome tables.

## The Godot client and bridge an editor would build on

**`apps/client/` — one Godot 4 project, combat slice only**

- `project` (`apps/client/project.godot:1`): the client project; `apps/editor` — none found.
- `extension file` (`apps/client/crpg.gdextension:1`): points at `crpg_godot.dll` with entry symbol `gdext_rust_init` and `compatibility_minimum = 4.7` (`docs/architecture/crpg-godot.md:138`).
- `Main scene` (`apps/client/scenes/Main.tscn:1`): root scene opened by `Godot4 --path apps/client`.
- `Main.gd` (`apps/client/scripts/Main.gd:1`): owns one `CrpgSession`, calls `open`, one `poll()` per frame, forwards to `Proxies` and `Hud`.
- `SceneSync.gd` (`apps/client/scripts/SceneSync.gd:1`): reconciles view rows to `Node3D` proxies; holds no revision state.
- `CombatHud.gd` (`apps/client/scripts/CombatHud.gd:1`): renders round, turn, health, options, log and end block; decides no rule.
- `load.gd` (`apps/client/tests/load.gd:1`): the E1 DLL/class load proof.
- `smoke.gd` (`apps/client/tests/smoke.gd:1`): the E2 scene-and-HUD end-to-end check.
- `hud.gd` / `scene_sync.gd` (`apps/client/tests/hud.gd:1`, `apps/client/tests/scene_sync.gd:1`): unit runners for the GDScript case files.
- `Client scope` (`docs/architecture/crpg-godot.md:167`): "the terminal-free slice"; options and ids come from the bridge; no rule is decided in the scene.
- `No input` (`docs/architecture/crpg-godot.md:259`): "Camera control and picking are T050 and deliberately absent."

**`crates/crpg-godot/` — the bridge surface**

- `crate doc` (`crates/crpg-godot/src/lib.rs:3`): "Godot presentation bridge (ADR-0033)"; only crate allowed to depend on `godot` or carry `unsafe`.
- `modules` (`crates/crpg-godot/src/lib.rs:12`): `pub mod core`; `shell` is compiled only with the off-by-default `engine` feature (`crates/crpg-godot/src/lib.rs:14`).
- `BridgeCore::open` (`crates/crpg-godot/src/core/mod.rs:99`): `open(campaign_dir: &str, encounter_id: &str, player_side: &str, seed: i64) -> BridgeCode`.
- `BridgeCore::view` (`crates/crpg-godot/src/core/mod.rs:116`): `pub fn view(&self) -> Rc<SessionView>`.
- `BridgeCore::last_error` (`crates/crpg-godot/src/core/mod.rs:121`): `pub fn last_error(&self) -> &str`.
- `BridgeCore::poll` (`crates/crpg-godot/src/core/mod.rs:126`): `pub fn poll(&mut self, now_ms: u64) -> BridgeCode`.
- `BridgeCore::submit_option` (`crates/crpg-godot/src/core/mod.rs:135`): `pub fn submit_option(&mut self, revision: i64, index: i64) -> BridgeCode`.
- `BridgeCore::close` (`crates/crpg-godot/src/core/mod.rs:147`): `pub fn close(&mut self)`.
- `SessionView` (`crates/crpg-godot/src/core/client.rs:28`): revision, net ids, names, sides, placements, health, max health, health-known, dead, active, round, ended, option labels/targets, events, receipts.
- `Core modules` (`docs/architecture/crpg-godot.md:28`): `core/mod.rs` (`BridgeCore`, `SessionView`, `ViewEvent`, `ViewReceipt`, `ViewValue`), `core/errors.rs` (20 `BridgeCode`s, fatal 50–55), `core/link.rs` (`LocalLink`), `core/client.rs`, `core/authority.rs`.
- `View contract` (`docs/architecture/crpg-godot.md:103`): every slice has `net_ids().len()` entries; missing rows produce `""`; unknown health is `(0, 0, known 0, dead 0)`.
- `Fence policy` (`docs/architecture/crpg-godot.md:110`): a fatal fault records its code, fences and shuts the host down; only `close` clears a fence.
- `FFI pins` (`docs/architecture/crpg-godot.md:124`): `godot = "=0.5.5"` with `api-4-7`, optional behind `engine`; `cdylib` + `rlib`.
- `The one unsafe` (`docs/architecture/crpg-godot.md:151`): `unsafe impl ExtensionLibrary` in `shell.rs`, the crate's only hand-written `unsafe`.
- `ADR-0033` (`docs/adr/0033-godot-bridge.md:1`): "The Godot bridge: FFI surface, threading and the embedded host"; status Proposed in the file's header (`docs/adr/0033-godot-bridge.md:4`).
- `Bridge has no edit surface` (`crates/crpg-godot/src/core/mod.rs:99`): the only open inputs are a campaign directory, an encounter id, a side and a seed; no document, command or asset call exists in the bridge.

## Dialogue, quests, events and scripting today

**Documents that exist**

- `Dialogue` (`crates/crpg-data/src/document.rs:155`): a dialogue document with nodes.
- `DialogueNode` (`crates/crpg-data/src/document.rs:176`): carries `body: DialogueBody`.
- `DialogueBody` (`crates/crpg-data/src/document.rs:191`): the internally tagged line/choice/jump/link/end variants, with a hand-written `Deserialize` (`crates/crpg-data/src/document.rs:547`).
- `Quest` (`crates/crpg-data/src/document.rs:239`): a quest document with states and transitions.
- `QuestState` (`crates/crpg-data/src/document.rs:260`): one named state with its journal text.
- `Trigger`/`triggers` (`crates/crpg-data/src/document.rs:354`): a `TriggersDocument` for an area.
- `EventGraph` (`crates/crpg-data/src/ir.rs:13`): the authored graph — entry trigger, nodes, edges, locals.
- `Node` / `NodeBody` (`crates/crpg-data/src/ir.rs:78`, `crates/crpg-data/src/ir.rs:92`): condition, action, branch, sequence, `Wait`, `CallScript`, `CallGraph`.
- `Wait semantics` (`docs/CRPG_ENGINE_SPEC.md:624`): a running graph must survive save/load; continuations are world-owned serializable IR state.
- `Semantic checks` (`docs/architecture/crpg-data.md:57`): validation checks dialogue reachability and quest completion today.
- `Action declarations` (`docs/architecture/crpg-data.md:293`): `ActionSignatureStore` is declaration-only, with content-derived identity and no runtime handler.

**Execution and scripting**

- `Binding slice only` (`docs/architecture/crpg-script.md:10`): "What exists today is only the first, synchronous slice."
- `No Lua or interpreter` (`docs/architecture/crpg-script.md:15`): "There is no Lua runtime, no graph interpreter, no wait/yield or continuation, no native extension loading."
- `ActionBindings` (`crates/crpg-script/src/bindings.rs:157`): the immutable trusted handler table bound to T029a declarations.
- `dispatch` (`crates/crpg-script/src/bindings.rs:409`): validates one call, runs one handler, applies bounded `CombatAction` proposals transactionally.
- `No Lua dependency` (`crates/crpg-script/Cargo.toml:8`): "no third-party runtime and no Lua"; deps are `crpg-core`, `crpg-data`, `crpg-sim`.
- `T029b` (`tasks/T029b.md:1`): the implemented synchronous-binding contract; it is the only task file among T029b and T052–T054.
- `T052` (`docs/IMPLEMENTATION_PLAN.md:282`): persisted continuation state for dialogue, quests and waits — planned, no `tasks/T052.md` exists.
- `T053` (`docs/IMPLEMENTATION_PLAN.md:283`): event IR interpreter with serializable `Wait`s — planned, no `tasks/T053.md` exists.
- `T054` (`docs/IMPLEMENTATION_PLAN.md:284`): dialogue engine and quest state machine — planned, no `tasks/T054.md` exists.
- `Visual graph and text editors` (`docs/IMPLEMENTATION_PLAN.md:346`): the graph compiler is T073 and the visual editor T074, both M8.

## Custom content

**What exists for assets**

- `assets.lock authority` (`docs/CRPG_ENGINE_SPEC.md:518`): "It records each source asset path, BLAKE3 content hash, and import settings so imports are reproducible."
- `assets.lock vs campaign.lock` (`docs/CRPG_ENGINE_SPEC.md:520`): `campaign.lock` holds exact package versions/checksums plus one digest of `assets.lock`; the packaged `manifest.json` owns hashes of final packaged content.
- `Asset directories` (`docs/CRPG_ENGINE_SPEC.md:503`): `assets/models/`, `textures/`, `audio/`, `fonts/`, and `assets/assets.lock`.
- `AssetRecord` (`crates/crpg-data/src/package.rs:82`): `{ hash: Digest, import: BTreeMap<String, DataValue> }` — "Source content digest; the library does not read the source asset."
- `AssetsLock` (`crates/crpg-data/src/package.rs:92`): `{ assets: BTreeMap<SourcePath, AssetRecord>, _note? }`, the source-asset authority.
- `assets_lock_digest` (`crates/crpg-data/src/package.rs:241`): the canonical digest `campaign.lock` records.
- `Document variant` (`crates/crpg-data/src/document.rs:462`): `crpg.assets-lock/1`.
- `E016 split` (`docs/architecture/crpg-data.md:37`): source hashes/import settings in `assets.lock`; package versions/checksums plus one digest in `campaign.lock`; packaged-byte authority is a future separate manifest.
- `Ambience membership` (`docs/architecture/crpg-data.md:58`): validation checks assets-lock membership for ambience.
- `Assets FFI object` (`docs/CRPG_ENGINE_SPEC.md:1025`): the planned bridge object is `Assets` with import/scan/thumbnail.
- `Asset format decision` (`docs/CRPG_ENGINE_SPEC.md:1750`): "glTF 2.0 in, Godot's import pipeline, content-hashed in `assets.lock`".

**What does not exist**

- `No importer` (none found): no `import`, `glTF`, `portrait`, `audio` or `VFX` asset-import function appears in `crates/*/src`; `AssetsLock` only records caller-supplied hashes and settings.
- `No packaging` (`docs/IMPLEMENTATION_PLAN.md:393`): `crpgc pack` and `.crpg` hashing/signing are T093, still planned; the packaging dialog is spec text only (`docs/CRPG_ENGINE_SPEC.md:1075`).
- `No manifest writer` (none found): no packaged `manifest.json` type or writer exists in `crates/`; the data doc calls it "future packaged-byte authority" (`docs/architecture/crpg-data.md:39`).

**Risk and research**

- `Spec risk 15` (`docs/CRPG_ENGINE_SPEC.md:1782`): "Asset pipeline complexity: rigging, retargeting, LODs, materials for hundreds of creatures"; mitigation is fixed animation slots, one humanoid rig standard, bought or CC0 packs, no customisation system.
- `Spec §19.2 #22` (`docs/CRPG_ENGINE_SPEC.md:1704`): "Packaging to `.crpg` with hashing and signing."
- `T105` (`docs/IMPLEMENTATION_PLAN.md:275`): asset sourcing per spec risk 15, before T047; compare licence-compatible glTF packs for one rig standard, animations and props; H-buy.
- `T107` (`docs/IMPLEMENTATION_PLAN.md:392`): research on where authors share work and how several mods combine; feeds T093/T094.
- `T099 custom-content angle` (`docs/IMPLEMENTATION_PLAN.md:299`): NWN haks, Creation Kit, Solasta/BG3 modding and Godot's glTF import, "bringing in models, portraits, audio and VFX, checking them against a rig standard, and packaging them".

## Agent log

- 2026-11-19 (UTC) · opencode-go/deepseek-v4.1-flash + T101 facts · Recorded the requirements, research, edit API, CLI, data, bridge, scripting and asset facts the T101 authoring UX ADR must rest on, with every citation checked on branch `task/T101-facts-f1`.
