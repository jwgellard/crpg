# ADR-0038: Authoring UX — personas, flows, layout, intent commands, asset import and usability gates

Date: 2026-11-21 (UTC)
Status: **Proposed** — filed by T101 (architect). The user accepts it
(H-accept, `docs/IMPLEMENTATION_PLAN.md:301`). No M6 contract is approved
before then (`tasks/DECISIONS-2026-10-16.md:108`). Open questions for the
user are in `tasks/T101.md`.

Numbering: ADR-0037 is reserved for the lane-1 QUIC mapping
(`tasks/T150.md:1747`) and ADR-0030 for T024q (`docs/adr/0033-godot-bridge.md:664`),
so this is the next free number.

## Context

D28j made authoring a research stage because "a major, major concern is both
agent and human usability" (`tasks/DECISIONS-2026-10-16.md:92`). D28l fixed
the audience: module authors first, agents equal, the user's own game after
them; defining a ruleset is an authoring task in its own right (`:135–136`).
D28m added the asset-import flow for custom content (`:152`). The Phase 6
bar is "create the MVP campaign without hand-writing JSON", proved by someone
"who has never seen the JSON" (`docs/CRPG_ENGINE_SPEC.md:1604`, `:1606`).
Inputs: the fact sheet `tasks/T101-facts.md`, the eight T099 reports
(`docs/research/T099/R8-deep.md` … `R15-deep.md`), the T100 baseline
(`docs/research/T100/baseline.md`) and the blind draft `tasks/T101-draft-b.md`.
Every engine fact below was re-checked on `task/T101-a1` at `68be7d3`.

### What is true today

- **Edit API (T058).** `EditCommand` is closed at nine variants
  (`crates/crpg-edit/src/command.rs:15–76`); ids are caller-supplied;
  semantic findings never block; lock documents and the manifest's `package`,
  `engine` and `requires` are read-only to commands; `crpg-edit` has no
  parser, no I/O, no clock and no id generator (`crates/crpg-edit/AGENTS.md`
  invariants 6–10; `docs/architecture/crpg-edit.md:79–106`). Opening needs a
  `LoadedCampaign`, so a campaign cannot be created from nothing through it.
- **CLI.** `crpgc` has eleven subcommands, none of them `apply`
  (`crates/crpg-cli/src/main.rs:110`). `new` covers creature, item, dialogue
  and quest only (`docs/architecture/crpg-cli.md:610`); `schema` prints 17 of
  the 23 schemas (`tasks/T100.md:47–51`); there is no help output
  (`tasks/T100.md:106`).
- **Data an author meets.** Stats are raw `Fx16_16` integers (a creature's
  might of 6 is stored as `393216`, `campaigns/fixtures/combat_basic/creatures/hero.json`);
  every `name` is a locale key (`crates/crpg-data/src/document.rs:112`);
  `campaign.json` `entry` names a world, area and spawn placement
  (`:45`); `campaign.lock` carries the BLAKE3 digest of `assets.lock`.
  Creatures and items have no model or portrait field, and there is no
  placeable-prop kind: a placement's `prefab` is a creature or item
  (`document.rs:330`, `:413–416`); the only asset reference in content is
  `Area.ambience` (`:97–100`). Dialogue `conditions` and quest transition
  `condition` are opaque strings (`document.rs:200`, `:213`, `:279`). The IR's
  `Trigger::Death` carries no subject (`crates/crpg-data/src/ir.rs:64–65`).
  `Fx16_16::from_str` parses decimal text exactly and refuses inexact values
  (`crates/crpg-core/src/fixed.rs:295–343`).
- **Assets.** Nothing imports; `AssetsLock` only records caller-supplied
  hashes (`crates/crpg-data/src/package.rs:82–92`). The bridge has no edit or
  asset call (`crates/crpg-godot/src/core/mod.rs:99`).

### What the T100 baseline measured

50 runs (5 models × 5 briefs × 2) on today's `crpgc` and schemas
(`docs/research/T100/baseline.md`): 45 valid and playable.

| Brief | valid / 10 | complete / 10 | timeouts | mean steps |
|---|---|---|---|---|
| campaign-extend | 10 | 10 | 0 | 17.4 |
| campaign-small | 9 | 9 | 2 | 71.7 |
| campaign-medium | 7 | 9 | 2 | 95.4 |
| campaign-large | 10 | 10 | 0 | 102.3 |
| ruleset-small | 9 | 10 | 1 | 64.2 |

**The `errors` column (0.0 everywhere) is a scorer defect, not a result.**
The scorer counts only tool calls named `bash` with a top-level
`metadata.exit` (`tools/agents/authorbench/authorbench_scoring.py:195–210`);
the Windows OpenCode event log names the tool `shell` and nests the exit code
at `state.metadata.metadata.exit`. Every row therefore has `crpgc_calls: 0`.
A recount over the stored logs (local, untracked:
`C:/crpg/dev/crpg-slots/authorbench/baseline/logs/*.jsonl`; tool `shell`,
nested exit, the scorer's own `crpgc` regex) gives, per run: campaign-extend
6.9 `crpgc` calls / 0.6 failed, campaign-small 43.7 / 9.5, campaign-medium
40.0 / 8.0, campaign-large 60.0 / 11.9, ruleset-small 39.6 / 8.2. Across the
50 runs the models wrote JSON by hand (688 `write`, 201 `edit` tool calls,
against 212 `crpgc new` calls) and ran `validate` 1,126 times. These
recounted figures are provisional until the scorer is fixed and re-run on the
same logs (M2).

Where the steps go, from the diagnostics `validate --json` returned during
the runs (same recount; counts are occurrences):

1. **Cross-document wiring.** `dangling entry spawn/area/world reference`
   (503), `dangling world area reference` (27), dangling outcome-table,
   participant and ruleset references (22). All five invalid final states
   carry `dangling_reference` on `campaign.json`'s entry; four of the five
   were timeouts that never wrote the area.
2. **Display text.** `locale key has no locale table to resolve against`
   (147); one invalid final state.
3. **Bookkeeping files.** Missing `campaign.lock` / `assets/assets.lock` /
   `campaign.json` (110), `assets lock digest mismatch` (39),
   `invalid_lock` (8): models hand-computed a BLAKE3 digest.
4. **Layout discovery.** `document kind does not match its storage path`
   and probe documents written to guess directories (`worlds/probe.json`).
5. **Raw numbers and ids.** `combat stat … is not a whole number` (raw
   `Fx16_16`), `invalid ULID length`, hand-invented ULIDs.
6. **Missing surface.** 39 `--help` attempts and invented subcommands
   (`package`, `publish`, `hash`, `version`); `crpgc new area` and
   `crpgc schema ruleset` refused.

None of these is authoring a game; all are bookkeeping that a tool can do.
This ADR's command layer is aimed at exactly these six classes, and M1–M6
re-measure it with the same benchmark.

## Decision

Items are normative and numbered so a contract can cite them and a test or a
walkthrough can prove them. Prefixes: P personas, V vocabulary, I information
architecture, C command layer, W plan wire format, K campaign and ruleset
creation, E event authoring, A asset import, D data prerequisites, F task
flows, M agent measurement, H human usability method, U per-package
acceptance criteria.

### P. Personas (D28l)

P1. **Module author (primary).** Knows CRPGs and probably another toolset
(NWN, Solasta, Foundry; spec risk 18); does not know JSON, ULIDs, fixed
point, JSON Pointer, Godot or this engine's file layout. Works in the GUI
editor. Success: builds the §17.2 campaign, then Greenhollow, without
reading data files.

P2. **Agent (equal).** An LLM with a shell and `crpgc`, or a tool-calling
agent given the same verbs. Knows JSON; does not know the layout, the locale
convention, the lock digests or which ULIDs exist. Success: the T100 briefs,
measured by M1–M6.

P3. **The user's own game (after them).** Served by the same surfaces; no
decision here favours it over P1/P2.

P4. **Equality rule.** Every authoring capability exists first as a
command-layer operation (C) and a `crpgc` form (W); a GUI feature that has no
operation is not accepted (spec Phase 6 risk, `docs/CRPG_ENGINE_SPEC.md:1607`).
P1 and P2 differ only in presentation: the GUI emits the same operations an
agent writes (U-T103-1 proves it).

P5. **Agents are not usability evidence for people** (R11,
`docs/research/T099/R11-deep.md:15`). Agent runs may screen a design (H3);
only H1/H2 sessions accept a human criterion.

### V. Vocabulary authors see

V1. The GUI never shows a ULID, a file path, a JSON Pointer, a raw
fixed-point integer, a locale key or a Godot term (spec §11.4,
`docs/CRPG_ENGINE_SPEC.md:1031`). ULIDs are reachable only through a "Copy
id" action.

V2. Data terms map to author terms as follows; T102 uses these words and
every M6 contract keeps them.

| Data | Author sees |
|---|---|
| `campaign.json` `entry` spawn placement | Player start |
| placement | (the placed thing's own name) "placed in <area>" |
| locale key + `locale/en.json` string | Text (the string itself) |
| `slug` | Short name (shown under the display name, editable) |
| `Fx16_16` value | A decimal number (`6`, `1.5`) |
| encounter | Fight |
| outcome table | Roll table |
| ruleset | Rules |
| event graph authored as a page (E1) | Event ("When … if … do …") |
| `assets.lock` entry | Asset |
| diagnostic | Problem |
| `prefab` | (never shown: authors place a creature, item or prop) |

### I. Information architecture

I1. **Main window** is spec §11.4's (`docs/CRPG_ENGINE_SPEC.md:1033–1052`):
menu strip `Campaign ▾ Edit ▾ World ▾ Rules ▾ Test ▾ Package ▾ ▶ Play ⏹`;
left CAMPAIGN explorer; centre tabbed DOCUMENT AREA; right PROPERTIES over
REFERENCES; bottom PROBLEMS | CONSOLE | PLAYTEST LOG | SEARCH.

I2. **Explorer** is built from the campaign index, grouped by what authors
make, never by directory: Worlds ▸ areas; Creatures; Items; Props (if D2a,
see `tasks/T101.md` Q1); Dialogue; Quests; Fights; Events; Factions; Rules ▸
rules, abilities, roll tables, effects; Assets; Text. Each row shows the
display text, then the short name. "New …" on a group runs the matching
`create` (C3) and opens the result.

I3. **One tab per object or aggregate.** An area tab spans `area.json`,
`placements.json` and its triggers; a dialogue tab is the whole dialogue.

I4. **Properties** are generated from the schema (spec §11.4 item 3) using
the D1 annotations: a reference field is a picker filtered by kind, a text
field edits the string in the current locale, a fixed-point field is a
decimal box. No hand-written form exists for a field a schema can describe.

I5. **References** lists "Used by" and "Uses" for the selected object from
`explain_object` (`docs/architecture/crpg-data.md:112`), each row clickable.

I6. **Problems** renders `validate()` after every committed operation. Each
row: severity, the object's display text, the field's schema title, the
message, and a **Fix** button when the hint table (C9) has a fix for the
code. Clicking opens the object's tab at the field. The panel is the first
view built and polished (R8, `docs/research/T099/R8-deep.md:7`).

I7. **Undo/redo** is `CampaignDocument::undo/redo`; one operation list (one
GUI gesture, or one plan) is one undo step.

I8. **Apply plan…** (Campaign menu) loads a `crpg.plan/1` file (W1), shows
the dry-run receipt (changed objects in author terms, new problems) and
applies it as one undo step on confirmation. This is how an agent's work
enters an open editor without a second writer.

I9. **External changes.** When files under the open campaign change on disk,
an editor with no unsaved operations reloads; one with unsaved operations
shows both choices (reload and lose, or keep and overwrite on save) and does
nothing until the author picks.

I10. **Play never forces a save.** It plays the document's current canonical
bytes (T068 may stage them in a temporary directory). It is refused only when
the campaign cannot load or has no valid player start; other problems show
as a banner in the play view.

### C. The intent command layer (T103, `crpg-edit`)

C1. **Above, not instead (D28j).** A module `intent` in `crpg-edit` compiles
an ordered list of operations into `EditCommand`s and applies them with one
`apply_batch`. `EditCommand`, its invariants, caps, receipts and undo are
unchanged except for the one addition in A6.

C2. **Pure compile.** `compile(doc: &CampaignDocument, ops: &[Op], fresh:
&mut dyn Iterator<Item = Ulid>) -> Result<Compiled, OpError>` reads only
`doc` and `ops`; it pulls new ids from `fresh` in a pinned order (operation
order, then within an operation in authored field order) and never generates
one (invariant 8). `Compiled` holds the `EditCommand`s and the handle map
(C4). `apply_ops(doc, ops, fresh) -> Result<OpReceipt, OpError>` is compile
plus `apply_batch`; `OpReceipt` is `{ receipt: CommandReceipt, handles:
BTreeMap<String, Ulid> }`. T103 pins the exact names; these shapes are the
floor.

C3. **Thirteen operations**, under R7's twenty
(`docs/research/T099/R10-deep.md:42`). Each compiles to the commands listed.

| Op | Does | Compiles to |
|---|---|---|
| `create` | a new object from its kind's recipe (C5), optionally under a parent | `CreateDocument` and/or `InsertValue`, plus locale and parent-membership edits |
| `delete` | removes an object; blocked while used (C8) | `DeleteObject` / `RemoveValue`, plus detach edits when asked |
| `rename` | changes short name and/or display text | `SetValue` on `slug`, `RenameDocument` to the slug's path, locale `SetValue` |
| `set` | sets one field | `SetValue` |
| `add` | inserts into a list field | `InsertValue` |
| `remove` | removes a list member or optional field | `RemoveValue` |
| `text` | sets display text for a text field in one locale | locale `SetValue`/`InsertValue`, plus the key field if unset |
| `place` | places a creature, item or prop in an area | `PlaceInstance` |
| `move` | moves or turns a placement | `MovePlacement` |
| `start` | makes a placement the player start | `SetValue` of `entry` `{world, area, spawn}` together |
| `connect` | links dialogue nodes, or quest states by a transition | `SetValue`/`InsertValue` on `next`/`transitions` |
| `on` | creates an event page (E1) | `CreateDocument`/`InsertValue` of an `EventGraph` |
| `import` | records an imported asset (A3) | `RecordAsset` (A6) |

C4. **References.** Wherever an operation or a reference-typed value (D1)
names an object it may use: `@name` (an object created earlier in the same
list with `as: "name"`), `<kind>:<slug>` (an existing object of that kind;
exactly one match or `unknown_ref`/`ambiguous_ref`, R5), or a 26-character
ULID. Stored data always holds the ULID. A handle may be bound once per list
(`handle_reused`). The receipt returns every handle's ULID.

C5. **Recipes: scaffold, never blank** (R8, `R8-deep.md:9`). `create` of
each kind yields an object that adds no structural failure and no new
diagnostic beyond those its own required references imply. Minimum recipes:
`world`; `area` (also its `placements.json`, and the area added to the
parent world's `areas`); `creature` (every stat the campaign's rules declare,
at a whole-number placeholder T103 pins per stat kind); `item`; `faction`;
`dialogue` (an entry NPC line and an end); `dialogue-line`,
`dialogue-choice`, `dialogue-end` (under a dialogue); `quest` (states
"Started" and "Completed", the second terminal, entry the first);
`quest-state`; `encounter` (participants from `with`, rules from the
campaign's one ruleset or `rules`); `ability`; `outcome-table`; `effect`;
`ruleset` (K3). A `name` given on `create` becomes the display text in the
default locale under a generated key that never changes afterwards.

C6. **Author units.** A value written to a field D1 marks as fixed-point is
a decimal number in the author's unit, parsed from its text with
`Fx16_16::from_str` (exact, never through a float; an inexact value is
`bad_unit` with the two nearest exact values as the hint). Positions are
metres (ADR-0036 U1). The receipt reports stored values in the same units.

C7. **Paths.** A field is an object-relative JSON Pointer (`/stats/might`,
T058's `EditTarget::Object`); R4 keeps pointers for leaves. An unknown field
is `unknown_field` with the closest schema field names as the hint.

C8. **Delete is blocked while used** (spec §11.4 References: "offers to fix
or block"): `delete` fails `referenced` and lists the users unless
`detach: true`, which also removes each referring list member and nulls each
nullable reference in the same batch; a required single reference cannot be
detached and still fails. (User question Q3.)

C9. **Errors and hints** (R2, `R10-deep.md:17`). An `OpError` is `{ op:
usize, code, pointer, message, hint: Option<String> }`; `pointer` points into
the operation list. Codes are a closed set T103 pins, at least:
`unknown_op`, `unknown_kind`, `unknown_field`, `unknown_ref`,
`ambiguous_ref`, `wrong_ref_kind`, `bad_value`, `bad_unit`,
`handle_reused`, `ids_exhausted`, `too_large`, `referenced`, `protected`,
and `edit` (a T058 `EditError`, carried with the failing command's index).
`EditError` and `crpg_data::Diagnostic` are unchanged; diagnostic hints come
from a code-to-hint table in `intent`, shown by I6 and by W4.

C10. **Read side stays where it is.** `validate`, `explain` and `schema`
are not operations; R7's count covers mutations only.

### W. The `crpg.plan/1` wire format and `crpgc apply` (T059)

W1. A plan is one JSON document: `{"schema": "crpg.plan/1", "ops": [ … ]}`.
Each op is an object whose `op` member names a C3 operation and whose other
members are that operation's named fields. Example (normative shape, T103
pins every field):

```json
{"schema": "crpg.plan/1", "ops": [
  {"op": "create", "kind": "creature", "as": "goblin", "slug": "goblin",
   "name": "Goblin", "set": {"/stats/might": 4, "/stats/health": 6}},
  {"op": "place", "area": "area:start", "what": "@goblin",
   "as": "goblin-spot", "at": [3, 0, 2]},
  {"op": "create", "kind": "encounter", "slug": "first-blood",
   "name": "First Blood", "with": ["placement:hero-start", "@goblin-spot"]}
]}
```

W2. The codec lives in `crpg-edit` beside `intent` and reads JSON only
through `crpg-data`'s reader (invariant 6). If no suitable public reader
exists, T103 stops and a `crpg-data` task adds it. Number literals in
fixed-point fields are handed to C6 as text.

W3. `crpgc apply <campaign-root> <plan-path|-> [--dry-run] [--json]`.
Ids come from host entropy (T059, `docs/architecture/crpg-edit.md:119`).
The run is all or nothing: either every file of the save plan (and every
imported asset, A4) is written, or nothing is. `--dry-run` compiles and
applies in memory and writes nothing. Exit 0: committed (or dry-run
succeeded), even with diagnostics; 1: refused, nothing written; 2: usage.

W4. Output with `--json` is one canonical object: `{"ok", "dry_run",
"handles": {name: ulid}, "changed": [{"path", "kind"}], "diagnostics": [ …
each with "hint" … ], "error": OpError|null}`, bounded to changed paths and
diagnostics (R6). Without `--json`, one line per change and per problem.

W5. `crpgc help [<command>|op <op>]` prints each command's usage and each
operation's 3–4 sentence description with one example (R7); every example in
it is applied by a test against a template campaign and must succeed.

W6. `crpgc schema` accepts every document kind's stem (all 23 files, plus
`plan`).

### K. Creating campaigns and rulesets

K1. **No campaign starts blank.** `crpgc new campaign <dir> --template
<name> --slug <s> --name <text>` (and Campaign ▸ New…) writes a complete,
valid, playable campaign: one world, one area `start`, a hero creature
placed as the player start, a copy of the template's rules, locale, variables
and both lock files, all with fresh ids. This is a host operation outside
`crpg-edit` (C1 cannot open nothing).

K2. Templates ship with the engine and are listed by `crpgc help new`:
`starter-d6` (rules `minimal-d6`) and `starter-srd` (rules `srd-lite`), built
from `rulesets/` and checked by a test that each validates and plays under
`crpgc play` with scripted answers.

K3. **New rules from a template** (D28l, T110 GUI; T103 operation first):
`create` with `kind: "ruleset"` and `from: "<template-ruleset>"` copies the
template's ruleset, abilities, roll tables and effects into the campaign
with fresh ids, internal references remapped, a new package name from the
slug, and the template's package and version recorded as its origin
(Q2; the field is a T110 data item). The copy is independent: later changes
to the template do not reach it.

K4. **Per-campaign override** of an existing ruleset stays spec §11.4's diff
in `rules/overrides.json` (`docs/CRPG_ENGINE_SPEC.md:1070`); its format is
T110's. K3 and K4 are different things and the GUI names them differently
("New rules from template…", "Change these rules for this campaign…").

### E. Event authoring (the section T053 waits on)

E1. **Pages before graphs** (R12, `docs/research/T099/R12-deep.md:5`,
`:29`). The authoring unit for M6 is an event page: one trigger with its
subject ("When Goblin dies"), zero or more conditions ("if quest Main is
Started"), and an ordered list of actions ("set quest Main to Completed").
The `on` operation and the GUI's Events editor write one page as one
`EventGraph` whose nodes are conditions then actions in sequence.

E2. **Requirements on T053's IR and interpreter** (each a contract item):
(a) a page as E1 stores it is a valid `EventGraph` and runs; (b) a trigger
names its subject (the dying creature's placement, the area entered, the
dialogue node), either as trigger arguments or as a first condition on the
event's subject — T053 picks, but the page must read as one line; (c)
conditions and actions are calls against declared signatures
(`ActionSignatureStore`, `docs/architecture/crpg-data.md:293`), so forms are
generated from them; (d) evaluation order is authored order; (e) no polling:
triggers fire on events, waits are bounded `Wait` nodes; (f) a bad call is a
diagnostic with a pointer (`validate_call`'s code and pointer).

E3. **Typed conditions in dialogue and quests.** Dialogue `conditions` and
quest transition `condition` become typed calls under E2(c); opaque strings
cannot be checked or given a form. This is a schema change owned by T054's
contract (or a `crpg-data` task before it).

E4. The MVP quest completes by a page "When <enemy> dies → set quest to
Completed" (Q5), not by a hardcoded hook; spec §17.3 allows either.

E5. The visual graph editor stays M8 (T073/T074); a page is a valid graph,
so M8 opens pages without conversion.

### A. Asset import (D28m)

A1. **One flow, two entry points.** GUI Assets ▸ Import… and the plan
operation `import {"file", "as", "class"}`, classes `model`, `texture`,
`portrait`, `audio`, `font` (MVP: `model` and `texture`).

A2. **Check before record.** The host reads the source file and runs the
checks for its class as pure functions over bytes, owned by `crpg-data`
(no I/O there; `package.rs:82`'s rule is about reading files, not bytes):
glTF 2.0 structure and, for creature models, conformance to the one rig
standard T105 chooses (bone names and hierarchy from the glTF JSON chunk;
no transform or scale on skeleton nodes, R15 `R15-deep.md:5–6`); image
dimensions for portraits and complete texture sets (`:11–12`). Errors refuse
the import with a pointer into the source; warnings are recorded. The checks
must not depend on Godot (Linux headless, `AGENTS.md`); any new external
dependency needs its own approval.

A3. **Record.** A passing import becomes one `RecordAsset` command (A6)
writing the `assets.lock` entry (source path, BLAKE3 hash, import settings)
and the matching `campaign.lock` digest in the same step, so locks are never
hand-edited and every import is one undo step.

A4. **Copy.** The host copies the source into `assets/<class-dir>/<as>.<ext>`
as part of the same all-or-nothing write as the documents (W3). Undoing the
record leaves the file on disk unrecorded; Problems lists it as unused.

A5. **Use and remove.** An asset is referenced from content (D3) through
`set`/`place`; removing one is `delete` with C8's rule. Godot's import cache
and thumbnails are derived, never committed and never authority.

A6. **The one `EditCommand` addition.** `RecordAsset { path: SourcePath,
record: Option<AssetRecord> }` (`None` forgets) is added by T103. The
generic commands cannot express it because lock documents are protected
(`docs/architecture/crpg-edit.md:93–95`); that is the T058 rule for adding a
typed command (`crates/crpg-edit/AGENTS.md`, Known traps).

A7. R15's 16-character name limit is not adopted: it is the Aurora resref
limit, not a constraint of this engine (alternatives).

### D. Data prerequisites (each a `crpg-data` task, S-set: schema bytes change)

D1. **Authoring annotations in the generated schemas:** which integer fields
are fixed-point, which string fields are locale keys, and which ULID fields
reference which kinds. Forms (I4), author units (C6), reference resolution
(C4) and `wrong_ref_kind` all read them. Keyword names are the contract's.

D2. **Props.** A placeable scenery kind `prop` (Q1, recommended) or items as
props; either way placements may reference it.

D3. **Appearance.** An optional model asset reference on creature, item and
prop (and the portrait reference when T057 needs it).

D4. **Terrain** stays T063's data task; its contract must keep one brush
stroke as one undo entry inside `MAX_HISTORY_BYTES` and `MAX_VALUE_BYTES`
(`crates/crpg-edit/src/document.rs:27–33`), which a per-cell JSON array may
not; T063 decides the storage.

### F. Task flows

Each flow lists the GUI path and the operations it emits; the operations are
also the agent's flow (P4). "Check" names what proves the step.

F1. **New campaign** (prerequisite of every §17.2 item). Campaign ▸ New… →
pick `starter-d6`, type a name → the explorer shows World ▸ Start, Hero,
Rules. Emits K1. Check: Problems empty; ▶ Play starts at once.

F2. **§17.2-1 Area, terrain, props.** Worlds ▸ World ▸ New area → name →
the area tab opens. Terrain tool (T063) → flat or raise/lower. Place tool →
drag props from the asset browser. Emits `create area`, terrain edits,
`place` ×N. Check: placements listed under the area; Problems empty.

F3. **§17.2-2 Player start, NPC, hostile.** The template's Hero is already
the player start; to move it, drag it (`move`), or right-click another
placement → Make player start (`start`). Creatures ▸ New creature ×2
("Elder", "Goblin"; `create creature`) → drag each into the area (`place`).
Check: References on the area lists three placements.

F4. **§17.2-3 Creatures use `minimal-d6`.** The creature tab's right side
shows the campaign rules' stat block (three attributes, health, the Strike
ability, its 2d6 roll table) in decimals; edit Might → `set /stats/might`.
Creatures carry no ruleset field; the campaign's rules apply (a campaign with
two rulesets shows a rules picker for the panel only). Check:
`crpgc explain creature:goblin` shows the same values.

F5. **§17.2-4 Dialogue with a quest-starting choice.** Dialogue ▸ New
dialogue (`create dialogue`, speaker Elder) → add a choice and a line
(`create dialogue-choice`, `create dialogue-line`, `connect`) → on the
choice, Actions ▸ Start quest Main (`add /…/on_select`). Text toggle shows
the indented form and back without change (spec §5.3). Check: Walk this
conversation (T054) reaches all three nodes; none greyed.

F6. **§17.2-5 Quest, completes on enemy death.** Quests ▸ New quest (Started,
Completed) → on Completed, "Completes when…" → Goblin dies (`on` page, E4).
Check: reachability shows Completed reachable.

F7. **§17.2-6 Press Play.** ▶ Play (I10) → in-process server and client in
the document area (T068). Check: playtest log shows the session; Stop
returns to editing with the document unchanged.

F8. **§17.2-7 Click-to-move.** Area tab → Bake navigation, walkable overlay
on one key (T064) → Play → click the ground (T050). Check: the overlay
shows the walkable region; the character walks there.

F9. **§17.2-8 Talk, quest starts.** Play → click the Elder → choose →
journal shows Main: Started (T057). Check: debugger quest watch.

F10. **§17.2-9 Fight.** Fights ▸ New fight with Hero and Goblin (`create
encounter`) or a fight that starts on approach (T108's mode rules) → Play →
attack. Check: combat log with rolls; Goblin dies.

F11. **§17.2-10 Quest completes.** Continues F10: the F6 page fires; journal
shows Completed.

F12. **§17.2-11 Save, quit, load.** This is the game's save during Play
(T052 state), not the editor's file save. Play → Save → Stop → Play → Load.
Check: the debugger shows the same quest state and positions (M7 proves
state identity).

F13. **§17.2-12 Separate server, two clients.** Not an M6 editor feature
(spec "Multiplayer testing" is Phase 10). For M7 the author saves and runs
`crpg-server` with the campaign and two clients from the command line, as
the M7 acceptance documents. Check: both clients show the same world.

F14. **New rules from a template** (D28l; T110 GUI, T103 op first). Rules ▸
New rules from template… → `minimal-d6` → name → the Rules tab shows the
copy (K3); add a stat, edit a roll table row, change Strike's damage (`set`,
`add`). Fights using it: pick the new rules. Check: validate clean; the
encounter dry-run (spec §11.4 encounter editor) or `crpgc play` runs a
fight with the new rules.

F15. **Change rules for this campaign** (K4, T110). Rules ▸ minimal-d6 ▸
Change for this campaign… → edit → stored as an override. Check: the
template's own files are unchanged.

F16. **Import a model and use it** (A). Assets ▸ Import… → pick a `.glb` →
class Model → checks pass or show problems → Goblin's Appearance picker
lists it (`import`, `set /appearance`). Check: the asset appears in
Assets; `assets.lock` and `campaign.lock` agree (validate clean).

F17. **An agent's plan in the open editor** (I8). Campaign ▸ Apply plan… →
review → Apply → Undo reverts all of it.

### M. Agent measurement (re-run of T100)

M1. **Candidate design.** `authorbench.py run --design intent-v1 --crpgc
<T059 build> --tools tools-intent.txt`, same five briefs, same
`models.txt`, `--repeat 2`; then `compare baseline intent-v1`. The prompts
are unchanged; `tools-intent.txt` lists `new campaign`, `apply`, `help`,
`validate`, `explain`, `schema`, `play` and the thirteen operations.

M2. **Scorer erratum first.** Before any comparison the T100 scorer counts
tool `shell` as well as `bash` and reads the nested exit
(`state.metadata.metadata.exit`), and the baseline is re-scored from the
stored logs with no new model runs; the re-scored `errors` replace the
provisional figures above. The local logs must be kept until then.

M3. **Pass rule for T103 + T059** (all must hold):
(a) `compare` reports no cell `worse`;
(b) valid and playable ≥ 48/50 overall and ≥ 9/10 for every brief
(baseline 45/50; campaign-medium 7/10);
(c) at most 1 timeout in 50 runs (baseline 5);
(d) mean steps per brief over its ten runs ≤ 70% of baseline for the four
from-nothing briefs — campaign-small ≤ 50.2, campaign-medium ≤ 66.8,
campaign-large ≤ 71.6, ruleset-small ≤ 44.9 — and campaign-extend ≤ 17.4;
(e) mean failed `crpgc` calls per run ≤ 50% of the re-scored baseline for
every brief;
(f) no final state carries `dangling_reference`, `missing_locale_key`,
`layout`, `invalid_lock`, `assets_lock_mismatch` or `malformed`.

M4. **Held-out brief.** A sixth brief, `campaign-story` (an NPC, a
dialogue whose choice starts a quest, a page completing it on a death), is
added and run on both designs; it is never used to write `help` text or
examples (R10, `R10-deep.md:57`). Pass: candidate valid and complete counts
≥ baseline's. (Spend: Q4.)

M5. A failed M3/M4 is a finding for T103, not a reason to relax the numbers;
changing a threshold needs the user.

M6. Each later candidate (for example after T061 forms) re-runs M1 with its
own design label against the same baseline.

### H. Human usability method (R11)

H1. **Think-aloud rounds** of five testers on the flows of a package (the
user first, then volunteer module authors with informed, opt-in consent,
`R11-deep.md:5`, `:13`). A flow passes when at least four of five finish it
without help and no finding rated "blocks the task" is open. Findings go back
to T101 (supersession entry) or to the package's contract.

H2. **Cognitive walkthrough** for first-run and novel flows (F1, F6, F14,
F16, F17) before H1 (`R11-deep.md:9`).

H3. **Heuristic screen** by three evaluators before each H1 round (agents
may be evaluators; P5).

H4. **Gates already in the plan:** H-proto on T102, the session after T062,
H-author at M6 exit, Greenhollow (T079).

### U. Acceptance criteria per package

Automated items are tests in the package's suite; H items use H1/H2.

- **T102 (prototype).** U-T102-1: clickable F1–F17 screens in V2's words.
  U-T102-2: H-proto by the user, H1 pass on F1–F6; findings recorded.
- **T103 (intent layer).** U-T103-1: for each C3 operation, a test runs the
  same operation as the GUI's typed call and as a `crpg.plan/1` op and gets
  equal `OpReceipt`s and equal canonical files. U-T103-2: every C5 recipe
  applied to each K2 template adds no new diagnostic. U-T103-3: compile is
  deterministic for a fixed `fresh` sequence and pulls ids in C2's order.
  U-T103-4: every C9 code has a test that produces it, and every hint names
  an operation that fixes it. U-T103-5: an F3–F6 plan for `starter-d6`
  yields a campaign that validates clean and plays.
- **T059 (`crpgc apply`).** U-T059-1: W3 all-or-nothing, proven by a failure
  injected between writes. U-T059-2: `--dry-run` writes nothing and prints
  the same receipt as the real run except ids. U-T059-3: every `help`
  example applies (W5). U-T059-4: M3 and M4 pass.
- **new campaign / templates (K1, K2).** U-K-1: each template validates and
  plays scripted. U-K-2: two runs give equal files except ids.
- **D1 annotations.** U-D1-1: every fixed-point, locale-key and reference
  field in the 23 schemas is annotated; a test walks the Rust shapes.
- **T060 (shell, explorer, problems).** U-T060-1: explorer groups per I2,
  no path or ULID visible. U-T060-2: a problem row opens its object at the
  field. U-T060-3: problems refresh after every operation. U-T060-4 (H1): a
  tester finds a named creature and fixes a dangling reference from
  Problems.
- **T061 (forms).** U-T061-1: adding a field to a document shape shows it in
  the form with no UI code. U-T061-2: fixed-point fields show decimals,
  references show pickers, text fields show strings. U-T061-3 (H1): a
  tester edits a creature's stats and name without seeing JSON.
- **T062 (placement).** U-T062-1: select, move, rotate, snap, duplicate,
  delete each emit one operation and undo byte-exactly. U-T062-2 (H1 in the
  post-T062 session): F2's props and F3's placements.
- **T063 (terrain).** U-T063-1: one stroke is one undo entry inside the
  history caps (D4). U-T063-2 (H1): F2's terrain step.
- **T064 (navmesh).** U-T064-1: bake and overlay toggle each one action; a
  walled-off region shows not walkable. U-T064-2 (H2): F8.
- **T065 (creature editor).** U-T065-1: the rules panel matches `explain`
  for the same creature. U-T065-2 (H1): F4.
- **T066 (dialogue editor).** U-T066-1: text toggle round trip is
  byte-identical on every dialogue fixture. U-T066-2: unreachable nodes grey
  within one operation. U-T066-3 (H1): F5.
- **T067 (quest editor).** U-T067-1: "Completes when…" writes an E1 page.
  U-T067-2: reachability matches `validate`. U-T067-3 (H1): F6.
- **T068 (Play).** U-T068-1: Play without saving runs the unsaved state
  (I10). U-T068-2: Stop leaves `canonical_files()` unchanged. U-T068-3 (H1):
  F7–F11 with at most one request for help per tester.
- **Apply plan… (I8).** U-I8-1: a plan applies as one undo step. U-I8-2
  (H2): F17.
- **Asset import (A).** U-A-1: a non-conforming rig is refused with a
  pointer; nothing is written. U-A-2: import then undo leaves both locks as
  before. U-A-3: the checks run in the Linux headless build. U-A-4 (H1): F16.
- **T053/T054 (events, dialogue engine).** U-E-1: an `on` page per E1 runs
  in the interpreter. U-E-2: E2(b)–(f) each have a test. U-E-3: E3's typed
  conditions validate with pointers.
- **T110 (rules editor).** U-T110-1: K3's copy validates and plays; the
  template's files are unchanged. U-T110-2: a K4 override changes only the
  campaign. U-T110-3 (H1): F14 and F15.
- **M6 exit.** H-author on F1–F12 by someone who has never seen the JSON.

## Authority and affected surface

- Decided by: Proposed; the user accepts (H-accept).
- Authoritative contracts: T103 (C, W1–W2, A6), T059 (W3–W6), the new
  campaign/template task (K1–K2), T053/T054 (E), the D1–D3 data tasks, T047
  follow-on or a new asset task (A), T060–T068 and T110 (I, U). This ADR
  fixes behaviour; each contract pins names and bytes.
- Owning crates: `crpg-edit` (intent, codec, `RecordAsset`), `crpg-cli`
  (`apply`, `new campaign`, `help`, `schema` stems), `crpg-data` (D1–D3,
  asset checks over bytes), `crpg-script` (E), `crpg-godot` and
  `apps/editor` (I), `rulesets/` and a templates directory (K2),
  `tools/agents/authorbench` (M2, M4).

## Compatibility, hash and dependency impact

- Persisted shapes: D1–D3 and E3 change generated schemas and document
  kinds (S-set, schema drift re-baseline under review). `crpg.plan/1` is new.
  `EditCommand` gains `RecordAsset` (T103 contract). Nothing else changes.
- Replay, `state_hash`, goldens: unchanged by this ADR; E3 and D2/D3 may
  touch loaded-content hashes when their tasks land, each under its own
  golden review.
- Dependencies: no new crate and no new external dependency is decided.
  `crpg-cli` → `crpg-edit` is a new edge for T059 (`ALLOWED` gives
  `crpg-cli` no row limit, `tools/lint/deps.py:70`; T059 records it).
  `crpg-server` stays excluded from `crpg-edit`.
- Platform (ADR-0012): every C, W, K, A2 check runs on Windows and Linux
  headless without Godot; the GUI is Windows.

## Alternatives considered

From draft b (`tasks/T101-draft-b.md`), kept: the persona order, the
three-region layout and its rules, scaffold-never-blank, generated forms,
the headless-first rule, the module placement of the layer in `crpg-edit`, a
separate error type above `EditError`, conformance-gated import, receipts
equal between GUI and agent (its T103 criterion), and T103 before T059.
Rejected or corrected:

- **A verb set that renames `EditCommand`.** Nine of its fourteen verbs map
  one-to-one to the nine commands; it would not remove any of the six
  measured failure classes (entry wiring, locale, locks, layout, raw
  numbers and ids, missing help). C3's operations each do the bookkeeping
  in one step.
- **`undo`/`redo`/`validate` as `apply` verbs.** History is in-memory and
  private (`crpg-edit` `history`); a CLI process has none to undo. The plan
  file plus version control is the agent's undo; `validate` is read-side.
- **A separate `link` verb.** C4 resolves references in any reference-typed
  value, so `link` duplicates `set`.
- **Schema field names instead of pointers.** Ambiguous for nested and list
  members; T058 already gives object-relative pointers, and R4 keeps
  pointers for leaves.
- **`plan` and `apply_plan` as verbs.** One file with `--dry-run` gives R3's
  reviewable step without a second representation.
- **Creature "ruleset field"** (its flow 3): creatures have none; the
  encounter and the campaign carry rules.
- **§17.2-11 as an editor save** (its flow 11): the item is the game's save
  and load.
- **`crpgc run` to verify item 12** (its flow 12): `run` never opens a
  campaign (`tasks/T100.md:38–46`).
- **New rules stored as a diff against the template** (its flow 13 and
  decision 9): that conflates K3 and K4; a new ruleset is a new package.
- **16-character asset ids** (its asset step 3): Aurora's resref limit.
- **Deferring event authoring** (its open question 9): T053 waits on it, so
  E gives the requirements now.
- **Clickable JSON Pointers in Problems**: V1.
- **Thresholds with no baseline** ("≤ 3 clicks", "≤ 2 minutes"): replaced
  by M3's measured targets and H1's completion rule.

Also rejected: a new `crpg-assets` crate now (a `deps.py` row and approval
before T105 has sized the checks; revisit at that contract); raw fixed-point
in forms and plans (fails the Phase 6 bar); live inheritance for new rules
(Q2); a hardcoded quest hook for the MVP (Q5).

## Consequences and follow-on contracts

| Contract | Crate | Must pin | Depends on |
|---|---|---|---|
| T100 erratum (scorer) + re-score | `tools/agents/authorbench` | M2 | none |
| D1 schema annotations | `crpg-data` (S) | D1 | accepted |
| D2 props, D3 appearance | `crpg-data` (S) | D2, D3 | Q1 |
| T103 intent layer (may split: ops, codec) | `crpg-edit` | C1–C10, W1–W2, A6, U-T103 | D1 |
| T059 `crpgc apply`, `help`, `schema` stems | `crpg-cli` | W3–W6, U-T059 | T103 |
| new campaign + templates | `crpg-cli`, templates data | K1–K2, U-K | T103 |
| M4 held-out brief + candidate run | `tools/agents/authorbench` | M1, M3, M4 | T059, Q4 |
| T053, T054 | `crpg-script` (+ data for E3) | E1–E5, U-E | accepted |
| asset import | `crpg-data`, `crpg-cli`, `crpg-godot` | A1–A6, U-A | T105, D3 |
| T102 prototype | doc-only | F1–F17, V2, U-T102 | accepted draft |
| T060–T068, Apply plan… | `crpg-godot`, `apps/editor` | I, U-T060…U-T068, U-I8 | T103, T059 |
| T110 | `crpg-edit`, `crpg-godot` | K3, K4, F14, F15, U-T110 | T108, M6 |

Easier: one surface for people and agents; agents stop doing bookkeeping.
Harder: four schema changes (D1–D3, E3) land before the editor tasks. Left
open: the time-mode authoring of fights (T108/ADR-0036), the AI authoring
surface (D65i), packaging and mod combination (T093, T107), and live edits
(T085).

## Acceptance and evidence

Not yet run. Evidence will be: the H-accept record; the M2 re-score and the
M3/M4 comparison in `docs/research/T100/`; each package's U items in its
completion record; H1 session notes per round.

## Supersession and corrections

## Agent log

- 2026-11-21 (UTC) · claude-code/opus-5.5 (architect) + T101 · Filed the authoring UX ADR as Proposed, adjudicating blind draft b against the code and the measured T100 baseline (including the scorer defect that zeroed its errors column), so T102, T103, T059, T053/T054 and T110 have numbered items to contract against.
