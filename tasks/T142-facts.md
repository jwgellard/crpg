# T142 facts — fact sheet for the `crpg.ui/1` contract (`crpg-data`)

Fact sheet for T142 (`crpg-data`: the `crpg.ui/1` schema, its validation and
generated defaults; ADR-0029 U1–U8 and U10). Facts only. Every `path:line`
resolves on branch `task/T142-facts-f1`.

## 1. Requirements

### ADR-0029 (`docs/adr/0029-kernel-range-and-time-model.md`)

- `U1` (`docs/adr/0029-kernel-range-and-time-model.md:353`) — "A ruleset may ship one `crpg.ui/1` document. It is presentation data: never loaded into a World, never hashed, delivered to clients in T072's presentation manifest. The server ignores it."
- `U2` (`docs/adr/0029-kernel-range-and-time-model.md:357`) — "No expressions, scripts, layout coordinates or widget names. Every binding is an authored id or name that resolves in the same ruleset (stat, pool, ability, effect, outcome label, tag) or campaign (item). Every label is a locale key (`crpg.locale/1`). Every icon is an asset reference validated by the asset pipeline (T101)."
- `U3` (`docs/adr/0029-kernel-range-and-time-model.md:363`) — "Stats: label, short label, description, group, and a format from a closed set (`integer`, `signed`, `fraction` of a stat or pool, `percent`, `boolean`, `enum` labels, `tags`, `dice`). Pools: label, icon, style (`pips`, `bar`, `number`), shown on the HUD or not. Abilities: label, description, icon, category tags, target hint, `show_odds`. Effects: label, icon, show duration. Outcomes: a label for each outcome including `custom:n`, which the combat log uses."
- `U4` (`docs/adr/0029-kernel-range-and-time-model.md:371`) — "Closed kinds: `character_sheet`, `action_bar`, `spellbook`, `inventory`, `level_up`, `combat_hud`, `character_creation`. A screen is an ordered list of sections. Section kinds are closed too: `stat_group`, `pool_group`, `ability_list` (explicit ids or a tag filter, with a sort), `effect_list`, `item_list` (tag filter), `timeline`, `log`, and `choice_steps`."
- `U5` (`docs/adr/0029-kernel-range-and-time-model.md:378`) — "A ruleset without a UI document, or a screen it omits, gets a default derived deterministically from the ruleset by a pure `crpg-data` function: all stats in declared order with slug labels, all abilities on the action bar, health, all pools and the timeline on the HUD."
- `U6` (`docs/adr/0029-kernel-range-and-time-model.md:385`) — "Unresolved ids, missing locale keys, unknown kinds and duplicate bindings are positioned `crpg-data` errors. They fail campaign validation but never server loading."
- `U7` (`docs/adr/0029-kernel-range-and-time-model.md:389`) — "Values come only from replicated facts (section 4). The client never evaluates a ruleset formula (spec §9.2). An ungranted field shows as hidden, never as zero."
- `U8` (`docs/adr/0029-kernel-range-and-time-model.md:393`) — "`level_up`, `character_creation` and `choice_steps` are reserved. They bind to progression and creation data families that do not exist yet (spec §3.4). Until then, the generated default omits them."
- `U9` (`docs/adr/0029-kernel-range-and-time-model.md:397`) — "`show_odds` displays the outcome distribution from a server-side read-only preview query (Follow-on 10), never client math (R16 recs 1 and 8)."
- `U10` (`docs/adr/0029-kernel-range-and-time-model.md:401`) — "T104 may add screen or section kinds through a schema version. It may not add code or expressions."
- Follow-on table row 9 (`docs/adr/0029-kernel-range-and-time-model.md:488`) — "| 9 | T142 | `crpg-data` | `crpg.ui/1` schema, validation and generated defaults (U1–U8, U10) | Accepted; T104 report as input | Yes |"
- Compatibility row (`docs/adr/0029-kernel-range-and-time-model.md:418`) — "`crpg-data` schema versions and migrations | **Yes** | Follow-on 3 (ruleset/ability/effect/encounter next versions with migrations), Follow-on 9 (`crpg.ui/1`)"
- T051 (`docs/adr/0029-kernel-range-and-time-model.md:17`) — "It gates T034 (movement ADR), T051 (HUD) and T109 (a third ruleset)."
- T072 (`docs/adr/0029-kernel-range-and-time-model.md:355`) — "delivered to clients in T072's presentation manifest."
- T104 (`docs/adr/0029-kernel-range-and-time-model.md:376`) — "The client draws each kind with its own widget (T104/T051)."
- T104 (`docs/adr/0029-kernel-range-and-time-model.md:401`) — "T104 may add screen or section kinds through a schema version."
- T051/T072 (`docs/adr/0029-kernel-range-and-time-model.md:466`) — "**T051** builds on U1–U10 and the N1 facts. **T072** carries the UI document."
- T143 (`docs/adr/0029-kernel-range-and-time-model.md:489`) — "| 10 | T143 | `crpg-sim` | Read-only action preview: exact integer outcome distribution, bounded (U9) | T137 | — |"
- T051 consume (`docs/adr/0029-kernel-range-and-time-model.md:410`) — "`crpg-godot` consumes (T051)."
- T072 carry (`docs/adr/0029-kernel-range-and-time-model.md:420`) — "The UI document reaches the client through T072; bridge queries are T045's"

### `docs/IMPLEMENTATION_PLAN.md`

- `docs/IMPLEMENTATION_PLAN.md:206` — "| T108 | A | `architect` (Opus 5.5) | ... (3) How a ruleset describes its player UI (character sheet, action bar, spellbook, inventory, level-up, combat HUD) so that any ruleset displays correctly, with T104. H-accept |"
- `docs/IMPLEMENTATION_PLAN.md:216` — "| T142 | C, S | `crpg-data` | Accepted; T104 report as input | `crpg.ui/1` schema, validation and generated defaults (ADR-0029 follow-on 9) |"
- `docs/IMPLEMENTATION_PLAN.md:256` — "| T051 | C → I | crpg-godot | T048, T104, T108 | HUD, combat log, `SimEvent`-driven VFX/audio hooks. Cut-down demo version has no T104/T108 prerequisite (ADR-0033). |"
- `docs/IMPLEMENTATION_PLAN.md:264` — "| T104 | R, then prototype | ... Player UX for turn-based CRPG combat, ... ruleset-driven screens (character sheet, action bar, spellbook, inventory, level-up), with T108. Then a clickable combat-HUD prototype. |"

### `tasks/BACKLOG.md`

- `tasks/BACKLOG.md:266` — "| T142 | `crpg-data` | contract to write; ready; T104 report as input | Yes | `crpg.ui/1` schema, validation and generated defaults (U1–U8, U10) | Accepted; T104 report as input |"

### Spec / HUD sentences (`docs/` grep `crpg.ui`, `show_odds`, `ui document`, `HUD`)

- `docs/CRPG_ENGINE_SPEC.md:865` — "UI               HUD, character sheet, inventory, dialogue, / combat log, timeline, spell book, journal" (client tree).
- `docs/CRPG_ENGINE_SPEC.md:1079` — "| **T2 Client-side** | UI themes, HUD layouts, model replacements, sounds | **None** | Never executes code. Data only. Never affects sim. Server-verified where it could confer advantage. |"
- `docs/CRPG_ENGINE_SPEC.md:1590` — "**Deliverables:** `crpg-godot` bridge, scene sync, camera, click-to-move with prediction, basic HUD, combat log, dialogue UI, `SimEvent`-driven VFX and audio hooks."

## 2. T104 input

Source: `docs/research/T104/R16-deep.md`. `docs/research/T104/README.md:7` records 23/32 confirmed, 9 paraphrase, 0 not found, 0 unavailable; no `[check: ...]` markers appear in `R16-deep.md`.

- `docs/research/T104/R16-deep.md:5` — "Show roll transparency (hit chance %, dice/modifiers) before and after actions." (odds display)
- `docs/research/T104/R16-deep.md:6` — "Why it applies: our data-driven rulesets must not hide math; players need trust in server-authoritative outcomes."
- `docs/research/T104/R16-deep.md:25` — "Display initiative/turn order explicitly (dice roll + modifiers)." (turn indicator)
- `docs/research/T104/R16-deep.md:26` — "Why it applies: server-authoritative multiplayer needs a shared, readable turn queue."
- `docs/research/T104/R16-deep.md:35` — "Offer optional auto-pause triggers in RTwP (end of round, low HP, enemy spotted)." (pause indicator)
- `docs/research/T104/R16-deep.md:36` — "Why it applies: lets players manage complex rulesets without reflex demands."
- `docs/research/T104/R16-deep.md:40` — "Show hit chance and health loss preview before committing an action." (odds display)
- `docs/research/T104/R16-deep.md:41` — "Why it applies: reduces misplays in multiplayer and supports learning."
- `docs/research/T104/R16-deep.md:45` — "Follow WCAG 1.4.1: never use color as the only visual means of conveying info." (screens/HUD)
- `docs/research/T104/R16-deep.md:46` — "Why it applies: baseline for all HUD elements across rulesets."
- `docs/research/T104/R16-deep.md:52` — comparison table header: "| Game | Camera | Action selection | Targeting preview | Turn-order display | Roll transparency | Log detail | URL |"
- `docs/research/T104/R16-deep.md:54` — "Baldur's Gate 3 | Free-floating; isometric to third-person | Actions/bonus actions/movement | Hit-chance % before attack | d4 initiative roll order | Hit-chance % visible | Not stated in fetched sources | ..."
- Defaults: none found in `docs/research/T104/R16-deep.md`.

## 3. Existing schema pattern

- Schema files: 22 files under `schemas/` (e.g. `schemas/ability.schema.json:1`, `schemas/ruleset.schema.json:1`), generated as draft-2020-12.
- Version-string constants: `crates/crpg-data/src/document.rs:401` — `#[serde(tag = "schema", deny_unknown_fields)]` on `Document`; variant tags at `crates/crpg-data/src/document.rs:405` (`crpg.campaign/1`) through `crates/crpg-data/src/document.rs:463` (`crpg.assets-lock/1`).
- Schema generation mapping: `crates/crpg-data/src/schema.rs:90` — "Generates exactly 22 filename-to-canonical-byte schemas without filesystem I/O."; `crates/crpg-data/src/schema.rs:94` — `pub fn generated_schemas()`.
- Schema registration list: `crates/crpg-data/src/schema.rs:107` — `document_schema!(Campaign, "crpg.campaign/1", "campaign.schema.json");` through `crates/crpg-data/src/schema.rs:142`.
- Loader entry point: `crates/crpg-data/src/loader.rs:251` — `pub fn load_campaign`.
- Read/write entry points: `crates/crpg-data/src/document.rs:486` — `pub fn read_document`; `crates/crpg-data/src/document.rs:493` — `pub fn write_document`.
- Validator entry points: `crates/crpg-data/src/validation.rs:243` — `pub fn validate_files`; `crates/crpg-data/src/validation.rs:884` — `pub fn validate`.
- Positioned-error type: `crates/crpg-data/src/validation.rs:148` — `pub struct Diagnostic` (file, pointer, severity, code, message, suggested_fix); `crates/crpg-data/src/error.rs:9` — `pub enum DataError`.
- `crpgc validate --json` reporting: `crates/crpg-cli/src/main.rs:37` — `VALIDATE_USAGE`; `crates/crpg-cli/src/main.rs:1538` — `fn validate_with_engine_version`.
- Migration registration: `crates/crpg-data/src/migrations/mod.rs:31` — `pub struct SchemaVersion`; `crates/crpg-data/src/migrations/mod.rs:61` — `macro_rules! define_registry`; `crates/crpg-data/src/migrations/mod.rs:104` — `pub fn schema_versions`; `crates/crpg-data/src/migrations/mod.rs:301` — `pub fn migrate_document`.
- Migration manifest: `crates/crpg-data/tests/fixtures/migrations.json:1` — rows with `schema_type`, `from`, `to`, `root`, `document`, `golden`.
- Ruleset ships optional document: no required-file entry for `rulesets/` (`crates/crpg-data/src/loader.rs:85` lists only `campaign.json`, `campaign.lock`, `assets/assets.lock`); `crates/crpg-data/src/loader.rs:130` — `Document::Ruleset(_) => family(p, "rulesets/")`; nearest existing example of an optional document kind is `crates/crpg-data/src/loader.rs:134` — `Document::Effect(_) => family(p, "effects/")`.

## 4. Generated defaults

- `crates/crpg-data/src/schema.rs:90` — schema generation, not document defaults.
- `crates/crpg-data/examples/generate_schemas.rs:15` — example writes baseline schemas.
- `crates/crpg-cli/src/main.rs:162` — `enum NewKind { Creature, Item, Dialogue, Quest }` scaffold types.
- `crates/crpg-cli/src/main.rs:1764` — `fn run_new` builds bounded Stage-2 authoring templates.
- `crates/crpg-data/src/combat.rs:86` — `#[serde(default, skip_serializing_if = "Option::is_none")]` on optional `effect`.
- `crates/crpg-data/src/combat.rs:359` — optional-missing real-time fields (`round_ticks`, `recovery_ticks`, `repeats_in_real_time`).
- `crates/crpg-data/src/migrations/ruleset.rs:22` — migration all-local defaults during `/1 -> /2`.
- Default document generation when a UI document is absent: none found.

## 5. Consumers

- `crpg.ui` / UI document references in `apps/client`: none found.
- `crpg.ui` / UI document references in `crates/crpg-godot`: none found.
- `crpg.ui` / UI document references in `crates/crpg-cli`: none found.
- `tasks/T051.md:1` — title names HUD; no `crpg.ui` or UI document reference found.
- T143 task file or row naming `crpg.ui` / UI document: none found.

## 6. Rulesets

### `rulesets/minimal-d6/`

Files shipped:
- `rulesets/minimal-d6/.gitattributes:1`
- `rulesets/minimal-d6/roll_under.json:1`
- `rulesets/minimal-d6/ruleset.json:1` — `"schema": "crpg.ruleset/1"`
- `rulesets/minimal-d6/strike.json:1`

Campaign copy:
- `campaigns/fixtures/combat_basic/rulesets/minimal-d6.json:1`

### `rulesets/srd-lite/`

Files shipped:
- `rulesets/srd-lite/.gitattributes:1`
- `rulesets/srd-lite/focus.json:1`
- `rulesets/srd-lite/focus_table.json:1`
- `rulesets/srd-lite/focusing.json:1`
- `rulesets/srd-lite/heavy.json:1`
- `rulesets/srd-lite/heavy_table.json:1`
- `rulesets/srd-lite/ruleset.json:1` — `"schema": "crpg.ruleset/2"`

Campaign copy:
- `campaigns/fixtures/combat_srd/rulesets/srd-lite.json:1`

## Agent log

- 2026-11-07 (UTC) · opencode-go/beam-free + T142 facts · Gathered cited facts for the crpg.ui/1 contract without designing it.