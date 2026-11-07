# T142 — `crpg.ui/1` schema, validation and generated defaults (`crpg-data`)

## Task

ADR-0029 Follow-on 9 (`docs/adr/0029-kernel-range-and-time-model.md:488`):
the `crpg.ui/1` document schema, its loading and validation (positioned
errors, as the existing document kinds do), how a ruleset ships one (U1),
and the defaults generated when a ruleset ships none. Requirements:
U1–U8 and U10 (`docs/adr/0029-kernel-range-and-time-model.md:353–402`).
U9 (odds preview) is T143, not T142: this contract names only the field
T142 must carry for it (`show_odds`).

## Crate(s)

**`crpg-data` only.** No dependency, no OS branch, no float, no
`HashMap`/`HashSet`.

Owns (full repository paths):

- `crates/crpg-data/src/ui.rs` (new module, re-exported at the crate root)
- `crates/crpg-data/src/document.rs` — `Document::Ui` variant
- `crates/crpg-data/src/loader.rs` — `ui/` family registration
- `crates/crpg-data/src/schema.rs` — `ui.schema.json` registration
- `crates/crpg-data/src/validation.rs` — `walk_ui` semantic pass
- `crates/crpg-data/src/migrations/mod.rs` — `crpg.ui` registry entry (v1, no edges)
- `schemas/ui.schema.json` (generated, 23rd schema)
- `crates/crpg-data/tests/ui.rs` (new integration suite)

goldens: none

## Decision

- **D1. A UI document is a first-class campaign document, `crpg.ui/1`,
  in its own `ui/` family**, not embedded in a ruleset and not a ruleset
  field. *Rejected:* embedding it in `Ruleset` (would force every ruleset
  document to carry presentation data and re-version `crpg.ruleset`; U1
  says it is presentation data that the server ignores, so it must stay
  separable). *Rejected:* a file inside the ruleset package (the loader
  has no package-relative document kind; all document kinds are campaign
  families, `crates/crpg-data/src/loader.rs:128–140`).
- **D2. The document names its ruleset via `ruleset: Ulid`**, a typed
  reference resolved in `validate`. *Rejected:* keying the file by ruleset
  slug (ids, not slugs, are the reference authority,
  `docs/architecture/crpg-data.md` §"Authorities and consumers"). *Rejected:*
  no reference at all (then "a ruleset ships one" cannot be checked).
- **D3. At most one UI document per ruleset.** A second is a
  `DuplicateBinding` diagnostic. *Rejected:* silently last-wins (would hide
  authoring mistakes; U6 forbids duplicate bindings).
- **D4. Per-object display metadata lists are optional-in-content, not
  optional-in-schema**: the vectors are required but may be empty, and any
  object not listed gets the deterministic default for that object.
  *Rejected:* required entries for every stat/pool/ability/effect/outcome
  (would make authoring a full sheet mandatory; U5's default guarantee
  must extend to partial documents).
- **D5. Bindings are names for stats/pools/outcomes and ids for
  abilities/effects.** Stats, pools and outcome labels are authored names
  (`String`); abilities and effects are `Ulid` references. *Rejected:*
  ids for stats too (stats have no ULID in `Ruleset.stats`,
  `crates/crpg-data/src/combat.rs:366–390`).
- **D6. `format` defaults to `integer` when omitted** (`#[serde(default)]`
  on `StatDisplay.format`). *Rejected:* required format (U6 does not list
  a missing format as an error, so it must default).
- **D7. Screens are an ordered `Vec<Screen>` with unique `kind`.** A screen
  kind may appear at most once. *Rejected:* a struct with one optional
  field per kind (U10 requires new kinds to be addable through a schema
  version; a closed struct field set cannot grow without a breaking
  change).
- **D8. A section is a tagged enum `Section`**, not a struct with a `kind`
  plus loose fields, so `additionalProperties: false` holds per kind
  (`crates/crpg-data/AGENTS.md` trap on unit variants). *Rejected:* one
  struct with all fields optional (would permit nonsense combinations).
- **D9. `AbilityList` carries `abilities: Vec<Ulid>` and
  `tag_filter: Option<Vec<String>>` plus `sort: AbilitySort`**; validation
  requires exactly one of `abilities` (non-empty) or `tag_filter` (Some).
  *Rejected:* allowing both (ambiguous resolution order).
- **D10. `Fraction` format names its target as a `FractionTarget` enum**
  (`Stat(name)` / `Pool(name)`), not a string. *Rejected:* a free string
  (would defer resolution to the client, violating U7).
- **D11. The default generator takes the ruleset plus its resolved ability
  documents** (`default_ui(ruleset, abilities)`), because ability labels
  need ability slugs. *Rejected:* ruleset alone (cannot produce locale
  keys for abilities without their slugs).
- **D12. Reserved kinds (`level_up`, `character_creation`, `choice_steps`)
  are parsed and validated but never defaulted** (U8). *Rejected:*
  generating empty placeholders (would imply the data families exist).
- **D13. `$id` is `https://crpg.dev/schemas/crpg.ui/1`** on the generated
  schema, added by the generator alongside the other 22 files.
  *Rejected:* no `$id` (consistent with today, but the contract needs a
  stable external identifier for T072's manifest).

## Interface

Module `crates/crpg-data/src/ui.rs`, re-exported at the crate root as
`crpg_data::ui::*` (mirroring `combat`). Constants:

```rust
pub const UI_SCHEMA: &str = "crpg.ui/1";
pub const UI_SCHEMA_FILE: &str = "ui.schema.json";
```

Types (all `Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema`,
`deny_unknown_fields` where structs; enums hand-written `Deserialize` only
where noted):

```rust
pub struct UiDocument {
    pub id: Ulid,
    pub slug: String,
    #[serde(rename = "_note", default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// The ruleset this UI describes (typed reference).
    pub ruleset: Ulid,
    #[serde(default)]
    pub stats: Vec<StatDisplay>,
    #[serde(default)]
    pub pools: Vec<PoolDisplay>,
    #[serde(default)]
    pub abilities: Vec<AbilityDisplay>,
    #[serde(default)]
    pub effects: Vec<EffectDisplay>,
    #[serde(default)]
    pub outcomes: Vec<OutcomeDisplay>,
    #[serde(default)]
    pub screens: Vec<Screen>,
}

pub struct StatDisplay {
    /// Declared stat name in the owning ruleset.
    pub stat: String,
    /// Locale key.
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default)]
    pub format: StatFormat,
}

pub enum StatFormat {
    Integer,
    Signed,
    Fraction { of: FractionTarget },
    Percent,
    Boolean,
    Enum,
    Tags,
    Dice,
}

pub enum FractionTarget {
    Stat(String),
    Pool(String),
}

pub struct PoolDisplay {
    pub pool: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub style: PoolStyle,
    #[serde(default)]
    pub show_on_hud: bool,
}

pub enum PoolStyle { Pips, Bar, Number }

pub struct AbilityDisplay {
    pub ability: Ulid,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default)]
    pub category_tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_hint: Option<String>,
    #[serde(default)]
    pub show_odds: bool,
}

pub struct EffectDisplay {
    pub effect: Ulid,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default)]
    pub show_duration: bool,
}

pub struct OutcomeDisplay {
    /// Outcome label, including `custom:n`.
    pub outcome: String,
    pub label: String,
}

pub struct Screen {
    pub kind: ScreenKind,
    pub sections: Vec<Section>,
}

pub enum ScreenKind {
    CharacterSheet,
    ActionBar,
    Spellbook,
    Inventory,
    LevelUp,
    CombatHud,
    CharacterCreation,
}

#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Section {
    StatGroup { group: Option<String> },
    PoolGroup { pools: Vec<String> },
    AbilityList {
        abilities: Vec<Ulid>,
        tag_filter: Option<Vec<String>>,
        sort: AbilitySort,
    },
    EffectList {},
    ItemList { tag_filter: Vec<String> },
    Timeline {},
    Log {},
    ChoiceSteps {},
}

pub enum AbilitySort { Declared, Name, Slug }
```

`ScreenKind` and `Section` use hand-written `Deserialize` enforcing exact
key sets per variant (the `Trigger`/`NodeBody` trap,
`crates/crpg-data/AGENTS.md`). `Serialize`/`JsonSchema` stay derived.

`Document` (`crates/crpg-data/src/document.rs:401`) gains, after `Effect`:

```rust
/// Authored player-UI description for a ruleset.
#[serde(rename = "crpg.ui/1")]
Ui(crate::UiDocument),
```

Loader (`crates/crpg-data/src/loader.rs:128–140`) gains:

```rust
Document::Ui(_) => family(p, "ui/"),
```

Schema registration (`crates/crpg-data/src/schema.rs:107–142`) gains:

```rust
document_schema!(UiDocument, "crpg.ui/1", "ui.schema.json");
```

Migrations registry (`crates/crpg-data/src/migrations/mod.rs:61`) gains:

```rust
"crpg.ui" => 1, edges: &[],
```

Generated default (pure function, no I/O, no clock):

```rust
pub fn default_ui(ruleset: &Ruleset, abilities: &[Ability]) -> UiDocument;
pub fn default_screen(kind: ScreenKind, ruleset: &Ruleset, abilities: &[Ability]) -> Screen;
```

`default_ui` produces: stats = one `StatDisplay` per declared stat in
declared order (label = stat slug used as a locale key, format = `integer`
unless the stat kind is `Fixed` → `fraction` of itself? see open
question), pools = one `PoolDisplay` per pool template (label = pool id,
style = `bar`, `show_on_hud = true`), abilities = one `AbilityDisplay`
per id in `ruleset.abilities` (label = the ability's slug + a locale-key
suffix, `show_odds = false`), effects = empty, outcomes = empty; screens:

- `character_sheet`: `[StatGroup { group: None }]`
- `action_bar`: `[AbilityList { abilities: ruleset.abilities.clone(), tag_filter: None, sort: Declared }]`
- `spellbook`: `[AbilityList { abilities: ruleset.abilities.clone(), tag_filter: None, sort: Declared }]`
- `inventory`: `[ItemList { tag_filter: vec![] }]`
- `combat_hud`: `[PoolGroup { pools: vec![] }, Timeline, Log]`
- `level_up`, `character_creation`: omitted (U8).

`default_screen` returns the same screen for one kind.

## Behaviour

1. **B1 (U1).** A campaign may contain zero or more `crpg.ui/1` documents.
   Each is loaded and validated like any other document; none is loaded into
   a `World` or hashed. `crpg-sim` never sees a `UiDocument`.
2. **B2 (U1).** `UiDocument.ruleset` must resolve to a `Ruleset` document
   in the same campaign; otherwise a `DanglingReference` diagnostic at
   `/ruleset`.
3. **B3 (U1).** At most one `UiDocument` may reference a given ruleset;
   a second produces a `DuplicateBinding` diagnostic at its `/ruleset`.
4. **B4 (U2).** Read-time (`Malformed`) rejects: an unknown `ScreenKind`
   or `Section` variant, an unknown `StatFormat`/`PoolStyle`/`AbilitySort`/
   `FractionTarget` variant, a non-string where a string is required, and
   any unknown field (deny_unknown_fields). No code, expression, coordinate
   or widget name is accepted anywhere in the schema.
5. **B5 (U2).** Every `label`, `short_label`, `description` and
   `target_hint` field is a locale key: validation reports `MissingLocaleKey`
   at the field pointer if it is absent from every locale document in the
   campaign.
6. **B6 (U2).** Every `icon` field is an asset reference: validation reports
   `MissingAsset` at the field pointer if the key is absent from the assets
   lock.
7. **B7 (U3).** `StatDisplay.stat` must name a declared stat of the
   referenced ruleset; otherwise `UnknownStat` at `/stats/<i>/stat`. A stat
   may appear at most once; a duplicate is `DuplicateBinding` at
   `/stats/<i>/stat`.
8. **B8 (U3).** `StatDisplay.format`'s `Fraction { of }` target must
   resolve: `FractionTarget::Stat(name)` to a declared stat, `Pool(name)` to
   a pool template of the ruleset; otherwise `UnknownStat`/`UnknownPool`
   at `/stats/<i>/format/of`.
9. **B9 (U3).** `PoolDisplay.pool` must name a pool template of the
   referenced ruleset; otherwise `UnknownPool` at `/pools/<i>/pool`. A pool
   may appear at most once (`DuplicateBinding`).
10. **B10 (U3).** `AbilityDisplay.ability` must be one of the referenced
    ruleset's `abilities`; otherwise `WrongReferenceKind`/`DanglingReference`
    at `/abilities/<i>/ability`. An ability may appear at most once
    (`DuplicateBinding`).
11. **B11 (U3).** `EffectDisplay.effect` must resolve to an `Effect`
    document; otherwise `DanglingReference` at `/effects/<i>/effect`. An
    effect may appear at most once (`DuplicateBinding`).
12. **B12 (U3).** `OutcomeDisplay.outcome` strings must be unique within
    the document; a duplicate is `DuplicateBinding` at
    `/outcomes/<i>/outcome`. (Resolution of outcome labels against the
    ruleset's outcome tables is an open question, Q4.)
13. **B13 (U3).** `Screen.kind` values must be unique within the document;
    a duplicate is `DuplicateBinding` at `/screens/<i>/kind`.
14. **B14 (U4).** `Screen.sections` may be empty. An empty `screens` vector
    is valid and means "all defaults".
15. **B15 (U4).** `AbilityList` validation requires exactly one of a
    non-empty `abilities` or a `Some` `tag_filter`; both or neither is a
    `Malformed`-reported? No — a positioned `DuplicateBinding`? Use
    `Malformed` read-time if both/neither are statically present? Since
    both are optional fields, make it a semantic `InvalidCost`-style
    diagnostic? Add code `InvalidBinding`? Simpler: read-time `Malformed`
    with message `ability_list must specify abilities or tag_filter, not
    both` (treated as structural). Actually because it's a cross-field
    rule, implement in `validate_local` as `Malformed` with path
    `/screens/<i>/sections/<j>`. I'll specify `Malformed` message
    `ability list must specify exactly one of abilities or tag_filter`.
16. **B16 (U4).** `ItemList.tag_filter` may be empty (means "all items").
17. **B17 (U5).** `default_ui(ruleset, abilities)` is pure and deterministic:
    same inputs produce byte-identical `UiDocument` and canonical JSON.
18. **B18 (U5).** A ruleset with no `UiDocument` referencing it, or a
    screen it omits, gets the default: `with_defaults` (or the loader's
    consumer) calls `default_screen` for each of the seven kinds not present
    in the document's `screens`, except `level_up` and
    `character_creation`, which are never defaulted (U8).
19. **B19 (U5).** The default is the single source used by the editor,
    agents and client; no second default implementation may exist.
20. **B20 (U6).** All findings above are `Diagnostic`s (collected, sorted by
    file/pointer/code/severity/message/fix) for `validate`, and never fail
    `load_campaign` or server loading.
21. **B21 (U7).** The schema contains no expression, formula or computed
    field; values are bindings only. (Enforced by review, not a test.)
22. **B22 (U8).** `default_ui` and `default_screen` never emit
    `LevelUp`, `CharacterCreation` or `ChoiceSteps` sections.
23. **B23 (U10).** New screen/section kinds are added by a new schema
    version (`crpg.ui/2`) with a migration edge; no code or expression is
    added. (This contract reserves the rule; no implementation here.)
24. **B24 (compat).** Adding `crpg.ui/1` does not change the bytes of any
    existing document or golden: no existing schema/family changes, and the
    migration registry gains a v1-only entry.

## Errors

Structural (`DataError`, read-time), all via `Malformed` where noted:

- `Malformed` message `ability list must specify exactly one of abilities or tag_filter`
  for `AbilityList` with both or neither.
- `Malformed` message `ability list sort is required` — never; `sort` is
  required (no default). (So a missing `sort` is serde's missing-field error.)
- Unknown enum variant messages are serde's standard `unknown variant`
  listing all variants (hand-written `Deserialize`).

Semantic (`DiagnosticCode`): add one new variant:

- `DuplicateBinding` → wire spelling `duplicate_binding`.

Reused codes: `DanglingReference`, `WrongReferenceKind`, `UnknownStat`,
`UnknownPool`, `MissingLocaleKey`, `MissingAsset`, `DuplicateSlug` (no —
use `DuplicateBinding` for all duplicate cases above).

Display texts for diagnostics follow the existing
`{code}: {message}` convention via `Diagnostic`'s serialized form; the
`message` strings are specified in Behaviour (B2–B13, B15).

## Tests

Named cases (in `crates/crpg-data/tests/ui.rs` unless noted):

- **T1 (B1, B2)** `ui_ruleset_reference`: a UI doc with a valid ruleset
  reference validates clean; a dangling reference yields one diagnostic at
  `/ruleset`.
- **T2 (B3)** `ui_one_per_ruleset`: two UI docs for one ruleset yield
  `DuplicateBinding` on the second.
- **T3 (B4)** `ui_unknown_kind`: unknown screen/section/stat-format/pool-style
  variant fails to deserialize (`Malformed`/serde error).
- **T4 (B5)** `ui_missing_locale_key`: a label not in any locale table yields
  `MissingLocaleKey`.
- **T5 (B6)** `ui_missing_asset`: an icon not in the assets lock yields
  `MissingAsset`.
- **T6 (B7)** `ui_unknown_stat`: a `StatDisplay` naming an undeclared stat
  yields `UnknownStat`.
- **T7 (B7)** `ui_duplicate_stat`: the same stat listed twice yields
  `DuplicateBinding`.
- **T8 (B8)** `ui_fraction_target`: a `Fraction` whose target is undeclared
  yields `UnknownStat`/`UnknownPool`.
- **T9 (B9)** `ui_unknown_pool`: a `PoolDisplay` naming an undeclared pool
  yields `UnknownPool`.
- **T10 (B10)** `ui_unknown_ability`: an `AbilityDisplay` for an ability not
  in the ruleset yields `DanglingReference`/`WrongReferenceKind`.
- **T11 (B11)** `ui_unknown_effect`: an `EffectDisplay` for a missing effect
  yields `DanglingReference`.
- **T12 (B12)** `ui_duplicate_outcome`: duplicate outcome strings yield
  `DuplicateBinding`.
- **T13 (B13)** `ui_duplicate_screen`: duplicate screen kinds yield
  `DuplicateBinding`.
- **T14 (B15)** `ui_ability_list_exclusive`: both/neither of abilities and
  tag_filter is rejected.
- **T15 (B17)** `ui_default_deterministic`: `default_ui` twice gives
  byte-identical canonical JSON.
- **T16 (B18, B22)** `ui_default_omits_reserved`: the default has no
  `level_up`, `character_creation` or `choice_steps`.
- **T17 (B20)** `ui_validation_nonfatal`: a UI doc with errors still loads
  (no `DataError`).
- **T18 (B24)** `ui_migration_registry`: `schema_versions()` includes
  `crpg.ui` at version 1 and `migrate_document` is a no-op identity on a
  `crpg.ui/1` document.
- **T19 (B4)** `ui_schema_drift`: `generated_schemas()` includes
  `ui.schema.json` and the checked-in file matches.
- **T20** `ui_round_trip`: a full UI document round-trips through
  `read_document`/`write_document` byte-identically.

Control for each: the same document with the offending field corrected
validates clean (for diagnostic tests) or deserializes (for read-time tests).

## What must not change

- No existing schema, family, or golden moves; `crates/crpg-data/tests/schema_drift.rs`
  count goes from 22 to 23 by addition only.
- `crpg.ruleset/2`, `crpg.ability/2`, `crpg.effect/1` unchanged.
- No new dependency; `deps.py` `ALLOWED` untouched.
- No `f32`/`f64`, no `HashMap`/`HashSet`, no `unsafe`.
- `validate` remains collected and non-fatal; UI errors never fail loading.

## Out of scope and hand-off

- **T143 (`crpg-sim`)**: read-only action preview for `show_odds` (U9).
  T142 only carries the `show_odds: bool` field.
- **T051 (`crpg-godot`)**: renders the screens/sections with its own
  widgets.
- **T072**: carries the UI document in the presentation manifest.
- **T104**: may add screen/section kinds via `crpg.ui/2` (U10).
- Progression/creation data families (`level_up`, `character_creation`,
  `choice_steps`) do not exist yet (U8); their defaults are omitted.

## Open questions (each with a recommendation)

1. **Default ability label locale key.** The ruleset carries only ability
   ids; labels need slugs. *Recommendation:* `default_ui` takes
   `&[Ability]` and uses each ability's `slug` as the locale-key stem
   (`ui.ability.<slug>`), as D11 states.
2. **Health on the HUD.** Health is a stat, not a pool; U5 says "health,
   all pools and the timeline on the HUD." *Recommendation:* default
   `combat_hud` includes a `StatGroup { group: Some("health") }` only when
   the ruleset declares a health stat, plus `PoolGroup { pools: [] }`,
   `Timeline`, `Log`. (If health is also a pool template, it appears once
   via the pool group.)
3. **Default format for `Fixed`/`Bool`/`Enum` stats.** *Recommendation:*
   `integer` for `Int`, `fraction` (of itself) for `Fixed`, `boolean` for
   `Bool`, `enum` for `Enum`, `tags` for `Tags`, `dice` for `Dice`; authors
   override via authored metadata.
4. **Outcome label resolution.** Whether `OutcomeDisplay.outcome` must name
   an outcome produced by the ruleset's outcome tables. *Recommendation:*
   defer to T143/the combat model; for T142 validate only uniqueness and
   locale keys, and add resolution when the outcome vocabulary stabilises.
5. **Effect ownership.** Whether an `EffectDisplay.effect` must be reachable
   from the ruleset. *Recommendation:* require only that it resolves to an
   `Effect` document; ownership is not tracked in data today.
6. **`AbilitySort` variants.** *Recommendation:* `Declared`, `Name`,
   `Slug` as listed; extend only by schema version.
7. **`$id` exact URI.** *Recommendation:* as D13; the manager may adjust
   to match the project's published schema base URL.
8. **Directory name `ui/` vs `uis/`.** *Recommendation:* `ui/`, matching
   the tag and the singular `locale/`.

## Agent log

- 2026-11-07 (UTC) · opencode-go/beam-free + T142 draft a · Blind draft of the crpg.ui/1 contract for crpg-data: schema, validation, ruleset shipping, generated defaults, following T138's exact-contract form. Nothing here is approved.