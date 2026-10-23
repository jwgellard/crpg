# T136b-draft-b — blind exact-contract draft for T136b (`crpg-data` surface schemas, ADR-0029 Follow-on 3)

Blind proposal by opencode-go/muse-spark-1.3-contributor. Nothing here is approved;
a Claude architect adjudicates it against the parallel draft, then the user approves.
Binding frame: `tasks/T136.md` §"T136b frame" and §"Versions and migrations per document".

## Task / Crate(s) / Status

ADR-0029 Follow-on row 3 (`docs/adr/0029-kernel-range-and-time-model.md:482`), minus the
T136a RT fields. Sole crate: `crpg-data` (plus its two docs and regenerated `schemas/`).
**S-set** (`tasks/T136.md:6-9`). Status: **proposed draft, unapproved**.
Assumes T136a (`round_ticks`, `recovery_ticks`, `repeats_in_real_time` at current
versions, `tasks/T136.md:35-44`) lands first; all field orders below are stated
on top of the T136a shapes.

## Decision

- **D1. Stat kinds grow to the six kernel kinds; derived stats are one optional
  expression wire.** `StatKindWire` (`crates/crpg-data/src/combat.rs:44-47`,
  today `Int` only) gains `Fixed`, `Bool`, `Tags`, `Dice` and
  `Enum { enum_id, variants }`, mirroring `StatKind`
  (`crates/crpg-rules/src/stats.rs:69-87`). `StatDecl`
  (`crates/crpg-data/src/combat.rs:52-57`) gains one optional-missing
  `derived: Option<ExprWire>` mirroring `Expr`
  (`crates/crpg-rules/src/stats.rs:96-113`) and pairing kind with formula as
  `StatDefinition` does (`crates/crpg-rules/src/stats.rs:117-124`). Rejected: a
  separate derived-stat family (splits what the kernel keeps paired per stat).
- **D2. Resolution is a bonus-stat list plus three new defense variants; no new
  threshold wire.** `Ability` gains optional-missing `bonus_stats: Vec<String>`;
  `DefenseWire` (`crates/crpg-data/src/combat.rs:105-113`) gains
  `Constant { value }`, `ActorStat { stat }` and `Opposed { dice, attribute }`
  (see Interface). Tiered grading reuses margin bands plus derived stats (the K4
  composition), so no threshold shape is added. Rejected: a full nested
  roll-request wire for opposed rolls (too large; a dice string plus an
  attribute covers the planned rulesets; the adapter builds `RollRequest`,
  `crates/crpg-rules/src/resolution.rs:281-299`).
- **D3. Damage dice sit beside the flat amount.** `DamageEntry`
  (`crates/crpg-data/src/combat.rs:266-271`) gains optional-missing
  `dice: Option<String>`; damage is `evaluate(dice) + amount`, so `amount: 0`
  with dice is a pure dice outcome. Rejected: replacing `amount` (breaks every
  existing entry and forces migration fills for no gain).
- **D4. Initiative is an optional spec; the authored constant stays.**
  `EncounterParticipant` (`crates/crpg-data/src/combat.rs:371-377`) gains
  optional-missing `initiative_spec: Option<InitiativeWire>` (`Stat` or `Dice`);
  absent means the authored `initiative`. Rejected: replacing `initiative`
  (every fixture and golden pins it, `tasks/T136.md:375-376`).
- **D5. Effects gain stat targets, tag grants and a lifetime enum; ops stay
  `Add`/`Set`.** `EffectTargetWire`
  (`crates/crpg-data/src/combat.rs:139-144`) gains `Stat { stat }`; `Effect`
  gains optional-missing `grants_tags` and `lifetime` replaces
  `duration_rounds` (`crates/crpg-data/src/combat.rs:212`) via migration.
  `Multiply`/`Clamp` (`crates/crpg-rules/src/modifier.rs:73-89`) are deferred:
  the wire `value` is `i32`-only and no consumer needs them (open question 4).
  Rejected: keeping `duration_rounds` beside `lifetime` (two spellings of one
  expiry), and adding ops now (unfunded generality).
- **D6. Sides and tags are optional participant labels; data sets no end
  condition.** `EncounterParticipant` gains optional-missing `side` and `tags`.
  Pairing, targeting and win conditions belong to the T137 adapter.
  Rejected: a `sides` table on `Encounter` (labels live with combatants).
- **D7. Migration steps are local-only and preserve unknown keys.** Each step
  follows the `v1_to_v2` precedent (`crates/crpg-data/src/migrations/ruleset.rs:14-35`,
  `crates/crpg-data/src/migrations/ability.rs:17-41`): check the source tag,
  mutate local fields only, rewrite the tag. Steps never strip keys their
  source version lacks (the T136a precedent, `tasks/T136.md:219-221`); the
  retagged typed decode rejects what the new version forbids. Rejected:
  cross-document lookups in steps (the per-type chain is context-free,
  `tasks/T136.md:306-323`).
- **D8. The T136 open question 3 becomes a semantic `Warning`, not a read-time
  failure.** `validate` gains one `Warning` (`crates/crpg-data/src/validation.rs:28-33`)
  for RT ability fields under a ruleset without `round_ticks`
  (`tasks/T136.md:215`); `push` is `Error`-only
  (`crates/crpg-data/src/validation.rs:411-428`), so the contract adds a
  `push_warning` helper. A `round_ticks` nobody needs stays clean.
  Rejected: `Malformed` (a file load must not fail for a mix T136a loads, `tasks/T136.md:133-136`).

## Interface

All additions live in `crates/crpg-data/src/combat.rs` unless named. New keys
are appended as the **last** fields of their structs (the T136a convention,
`tasks/T136.md:48-50`). Doc comments are verbatim: they become schema
`description` bytes.

```rust
pub enum StatKindWire {  // extends crates/crpg-data/src/combat.rs:44-47
    Int, Fixed, Bool, Tags, Dice,
    Enum { enum_id: String, variants: Vec<String> },
}

/// One typed stat value in authored form; mirrors StatValue (stats.rs:51-65).
pub enum StatValueWire {
    Int(i32),
    Fixed(i32),   // raw Fx16_16, as Creature stats persist whole values today
    Bool(bool),
    Enum { enum_id: String, variant: String },
    Tags(Vec<String>),
    Dice(String), // notation, 1..=128 UTF-8 bytes like ability dice
}

/// One typed derived-stat expression; mirrors Expr (stats.rs:96-113).
pub enum ExprWire {
    Literal(StatValueWire),
    Stat(String),
    Add(Box<ExprWire>, Box<ExprWire>),
    Subtract(Box<ExprWire>, Box<ExprWire>),
    Multiply(Box<ExprWire>, Box<ExprWire>),
    Divide(Box<ExprWire>, Box<ExprWire>),
    Min(Box<ExprWire>, Box<ExprWire>),
    Max(Box<ExprWire>, Box<ExprWire>),
}

pub struct StatDecl {  // extends combat.rs:52-57
    pub name: String,
    pub kind: StatKindWire,
    /// Derived formula evaluated before this stat's modifiers; absent means base stat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derived: Option<ExprWire>,
}

pub enum DefenseWire {  // extends combat.rs:105-113
    ActorAttribute,
    TargetStat { stat: String },
    /// Constant difficulty; `Against::Dc` value (resolution.rs:303-310).
    Constant { value: i32 },
    /// Fully modified actor stat; `Against::Stat` with entity = actor (resolution.rs:313-320).
    ActorStat { stat: String },
    /// Opposed roll; `Against::Opposed` (resolution.rs:323-324).
    Opposed { dice: String, attribute: String },
}

pub enum InitiativeWire {
    Stat { stat: String },   // rolled/derived initiative (tasks/T136.md:358-359)
    Dice { dice: String },   // rolled initiative expression
}

pub enum LifetimeWire {
    Rounds { rounds: u32 },   // expiry after this many rounds
    Encounter,                // expiry when the encounter ends
    UntilRest { rest: RestId }, // reuses RestId (combat.rs:75-80)
}
```

`StatValueWire`, `ExprWire`, `InitiativeWire` use derived
`Serialize`/`JsonSchema` with externally tagged JSON (`{"add":[lhs,rhs]}`,
`{"stat":{"stat":"might"}}`); `DefenseWire` and `LifetimeWire` keep
hand-written `Deserialize` impls enforcing exact key sets per variant (the
unit-variant trap, `crates/crpg-data/AGENTS.md:58-63`; precedent
`crates/crpg-data/src/combat.rs:560-588`). `Serialize`/`JsonSchema` stay
derived so schemas keep `additionalProperties: false`.

```rust
pub struct DamageEntry {  // extends combat.rs:266-271
    pub outcome: OutcomeWire,
    pub amount: u32,
    /// Optional dice expression added to `amount`; absent means flat damage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dice: Option<String>,
}

pub struct Effect {  // extends combat.rs:192-213; `duration_rounds` removed
    // ...id, slug, name, _note, aim, mod_type, policy, modifiers unchanged...
    /// Tags granted with the effect; absent means none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grants_tags: Option<Vec<String>>,
    /// Lifetime replacing `duration_rounds`; the sim owns expiry (no kernel effect store).
    pub lifetime: LifetimeWire,
}

pub struct EncounterParticipant {  // extends combat.rs:371-377
    pub placement: Ulid,
    pub initiative: i32,
    /// How initiative is determined; absent means the authored `initiative`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initiative_spec: Option<InitiativeWire>,
    /// Side label; absent means today's free-for-all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side: Option<String>,
    /// Combatant tags; absent means none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

pub struct Ability {  // extends combat.rs:276-315 on top of the T136a fields
    // ...existing fields including recovery_ticks, repeats_in_real_time unchanged...
    /// Bonus stat names added to the actor total; absent means none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bonus_stats: Option<Vec<String>>,
}
```

`EffectTargetWire` gains `Stat { stat: String }` (see B7). `EffectOpWire`
(`crates/crpg-data/src/combat.rs:147-154`) is unchanged. Tags throughout are
`String` names validated nonempty/unique at read time; the sim interns them
into `TagSet` (`crates/crpg-rules/src/stats.rs:22`).

Other `src/*.rs` changes: `document.rs` retags to `crpg.ruleset/3`,
`crpg.ability/3`, `crpg.encounter/2`, `crpg.effect/2`
(`crates/crpg-data/src/document.rs:432-444`); `schema.rs` mirrors the four tags
(`crates/crpg-data/src/schema.rs:116-124`); `migrations/mod.rs` gains four edges
(`crates/crpg-data/src/migrations/mod.rs:77-98`); new step functions per
§Migration steps; `validation.rs` gains one `DiagnosticCode` plus `push_warning` (B4).

## Migration steps

One step per document (frame: `tasks/T136.md:161-163`). Style follows
`migrations/ruleset.rs:14-35` and `migrations/ability.rs:17-41`. All four
steps keep the T136a fields byte-unchanged: `round_ticks` (ruleset),
`recovery_ticks` and `repeats_in_real_time` (ability) — present values
untouched, absent stays absent.

- **Ruleset `/2 → /3`** (`migrations/ruleset.rs::v2_to_v3`). Requires schema
  `crpg.ruleset/2`, else `Malformed "ruleset migration expects schema
  crpg.ruleset/2"`. Mutates no key: `stats` entries keep their `kind`
  (`"int"` decodes unchanged); `derived` is optional-missing so absent stays
  absent. Rejects: wrong source tag only; unknown keys are preserved and the
  retagged typed decode judges them. Retags to `crpg.ruleset/3`.
- **Ability `/2 → /3`** (`migrations/ability.rs::v2_to_v3`). Requires schema
  `crpg.ability/2`, else `Malformed "ability migration expects schema
  crpg.ability/2"`. Mutates no key: `bonus_stats`, `damage[].dice` and the new
  defense variants are optional-missing/additive. Rejects: wrong source tag
  only. Retags to `crpg.ability/3`.
- **Effect `/1 → /2`** (new `migrations/effect.rs::v1_to_v2`). Requires
  schema `crpg.effect/1`, else `Malformed "effect migration expects schema
  crpg.effect/1"`. Moves `duration_rounds: n` to
  `lifetime: {"type":"rounds","value":n}` and removes `duration_rounds`; if
  `duration_rounds` is absent but `lifetime` is present it keeps `lifetime`
  (forward-authored value, cf. D7); if both are present it fails `Malformed
  "effect migration has both duration_rounds and lifetime"`. Rejects: the
  both-present conflict and the wrong source tag. Retags to `crpg.effect/2`.
  `grants_tags` and `Stat`-targeted modifiers are additive and preserved.
- **Encounter `/1 → /2`** (new `migrations/encounter.rs::v1_to_v2`). Requires
  schema `crpg.encounter/1`, else `Malformed "encounter migration expects
  schema crpg.encounter/1"`. Mutates no key: `initiative` stays the fallback;
  `initiative_spec`, `side` and `tags` are optional-missing. Rejects: wrong
  source tag only. Retags to `crpg.encounter/2`.

`/1` ability/ruleset sources migrate the complete registered chain in memory
(`/1 → /2 → /3`), as today (`tasks/T136.md:314-316`). `rulesets/minimal-d6/`
(`/1` ruleset `tasks/T136.md:237`, `/1` ability) and `rulesets/srd-lite/`
(`/2` tags `tasks/T136.md:237-238`) stay byte-identical and migrate in
memory; e.g. `focusing.json`'s `duration_rounds: 2`
(`tasks/T136.md:285-292`) becomes `Rounds { rounds: 2 }`, semantically equal,
so replay and sim goldens stay unchanged (T137 proves end to end).

## Behaviour

1. **Decode.** Every new key is either absent or present. Present `dice`
   strings (ability opposed, damage, initiative) must be strings; present
   `value`/`rounds` integers must lie in range (zero then fails item 3);
   present `side`/`stat`/`enum_id` must be strings; `UntilRest` value must be
   a valid ULID (`RestId`, `crates/crpg-data/src/combat.rs:75-80`). Explicit
   `null` on an optional-missing key decodes as absent (the `natural_die`
   precedent, `crates/crpg-data/src/combat.rs:309-310`; `tasks/T136.md:216`).
   Unknown keys are still rejected (`crates/crpg-data/AGENTS.md:34-35`).
2. **Encode.** Absent optionals write no key. Canonical JSON sorts keys
   recursively, two spaces, one LF (`crates/crpg-data/AGENTS.md:40-42`). The
   canonical encoding of every existing document is therefore unchanged.
3. **Read-time checks** (`validate_combat_local`,
   `crates/crpg-data/src/combat.rs:703-879`, appended last in each arm so an
   earlier fault still reports first):
   - Ruleset: `enum_id` empty fails `Malformed "ruleset stat kind enum_id must
     be nonempty"`; empty/duplicate `variants` or empty entries fail `Malformed
     "ruleset stat kind enum variants must be nonempty and unique"`; a
     `derived` tree deeper than 32 fails `Malformed "ruleset derived expression
     is too deep"`, with more than 4096 nodes `Malformed "ruleset derived
     expression has too many nodes"` (kernel bounds
     `crates/crpg-rules/AGENTS.md:101-103`); a `Stat(name)` naming no declared
     stat of the same ruleset fails `Malformed "ruleset derived expression
     names an undeclared stat"`; a stat referencing itself fails `Malformed
     "ruleset derived stat must not reference itself"`.
   - Ability: duplicate/empty `bonus_stats` entries fail `Malformed "ability
     bonus stats must be nonempty and unique"`; an opposed `dice` outside
     `1..=128` bytes fails `Malformed "ability opposed dice must be 1..=128
     UTF-8 bytes"` (bound precedent `crates/crpg-data/src/combat.rs:771-774`);
     an empty opposed `attribute` fails `Malformed "ability opposed attribute
     must be nonempty"`; an empty `ActorStat`/`TargetStat`/`Constant` stat
     keeps the existing `Malformed "ability defense stat must be nonempty"`
     (`crates/crpg-data/src/combat.rs:804-808`); a `damage[].dice` outside
     `1..=128` bytes fails `Malformed "ability damage dice must be 1..=128
     UTF-8 bytes"`.
   - Effect: duplicate/empty `grants_tags` entries fail `Malformed "effect
     granted tags must be nonempty and unique"`; `Rounds { rounds: 0 }` fails
     `Malformed "effect lifetime rounds must be nonzero"` (nonzero precedent
     `crates/crpg-data/src/combat.rs:872-874`).
   - Encounter: empty `side` fails `Malformed "encounter participant side must
     be nonempty"`; duplicate/empty `tags` entries fail `Malformed "encounter
     participant tags must be nonempty and unique"`; an empty initiative
     `stat` fails `Malformed "encounter initiative stat must be nonempty"`;
     initiative `dice` outside `1..=128` bytes fails `Malformed "encounter
     initiative dice must be 1..=128 UTF-8 bytes"`.
4. **Semantic `validate`** (collected, positioned; orphan abilities skip
   ruleset-relative checks, precedent
   `crates/crpg-data/src/validation.rs:566-573`):
   - `bonus_stats[i]` not in the owning ruleset's `attributes` fails
     `unknown_stat` at `/bonus_stats/<i>` (precedent
     `crates/crpg-data/src/validation.rs:574-590`); naming a `Bool`/`Enum`/
     `Tags`/`Dice` stat fails `invalid_stat_value` at the same pointer.
   - `ActorStat`/`TargetStat` naming an undeclared stat or the health stat
     fails `unknown_stat` at `/defense` (precedent `:591-606`); `Opposed`
     `attribute` outside the owning ruleset's `attributes` fails
     `unknown_stat` at `/defense`.
   - `Stat`-targeted effect modifiers reached through an ability's `effect`
     must name a declared `Int` stat of the owning ruleset; otherwise
     `unknown_stat` (undeclared/health) or `invalid_stat_value` (non-`Int`)
     at the effect file's `/modifiers/<i>/target`, naming ability and effect.
   - Encounter prefabs: `missing_stat`/`invalid_stat_value` apply to
     `Int`/`Fixed` ruleset stats exactly as today
     (`crates/crpg-data/src/validation.rs:702-739`); other kinds are skipped
     (the sim supplies defaults; T137 owns them).
   - **Warning (T136 open question 3):** an ability carrying
     `recovery_ticks` or `repeats_in_real_time` whose owning ruleset has no
     `round_ticks` emits one `Warning` with a new code
     `rt_fields_without_round_ticks` at the ability file's `/recovery_ticks`
     (or `/repeats_in_real_time` when only it is present), message
     `"ability {ability} declares real-time fields but ruleset {ruleset} has
     no round_ticks"`, fix `"remove the real-time fields or add round_ticks
     to the ruleset"`. A `round_ticks` nobody needs stays clean
     (`tasks/T136.md:215`).
5. **Versions unchanged except the four bumps.** Tags and registry rows move
   per §Interface; `schema_versions()` reflects them. A `/1` ruleset/ability
   source migrates `/1 → /2 → /3` in memory before typed decode.
6. **Schemas.** After regeneration with the existing generator, exactly four
   files change: `ruleset.schema.json` (new `StatKindWire` variants,
   `derived`), `ability.schema.json` (`bonus_stats`, damage `dice`, new
   defense variants), `effect.schema.json` (`grants_tags`, `lifetime`),
   `encounter.schema.json` (`initiative_spec`, `side`, `tags`). None of the
   new keys is in `required`. The other 18 files are byte-identical.
7. **Kernel mapping.** Each shape maps onto an existing kernel type; data
   never depends on `crpg-rules` (copied convention,
   `crates/crpg-data/AGENTS.md:159-161`):
   - `StatKindWire` → `StatKind` (`crates/crpg-rules/src/stats.rs:69-87`);
     `StatValueWire` → `StatValue` (`:51-65`, incl. `EnumValue :32-37`);
     `ExprWire` → `Expr` (`:96-113`); `StatDecl` → `StatDefinition` (`:117-124`).
   - `DefenseWire::Constant` → `Against::Dc`
     (`crates/crpg-rules/src/resolution.rs:303-310`); `ActorStat`/`TargetStat`
     → `Against::Stat` (`:313-320`); `Opposed` → `Against::Opposed` (`:323-324`)
     with a `RollRequest` (`:281-299`) the adapter builds.
   - `bonus_stats` has no single kernel type: the adapter queries each named
     stat through the pipeline and adds the values to the actor total (a K4
     composition, cf. `docs/adr/0029-kernel-range-and-time-model.md:100-106`).
   - Damage/initiative/opposed `dice` → `DiceExpr`
     (`crates/crpg-rules/src/dice.rs:52-57`); validity bounds follow
     `DiceExpr::new` (`:71-82`); count-vs-`natural_die` stays T137's check.
   - `EffectTargetWire::Stat` → `ModifierTarget::Stat`
     (`crates/crpg-rules/src/modifier.rs:55-62`); `Roll`/`Dc` map as today;
     `EffectModifierWire` → `Modifier` (`:110-127`) with `Add`/`Set`
     (`:73-89`), `mod_type` → `ModTypeId` (`:35-36`), `policy` →
     `StackingPolicy` (`:131-139`); conditions stay `None` (`ConditionExpr`
     stays tag-only per K4, `docs/adr/0029-kernel-range-and-time-model.md:100-106`).
   - `grants_tags`/participant `tags` → `TagSet`
     (`crates/crpg-rules/src/stats.rs:22`) unioned into `QueryContext::tags`
     (`crates/crpg-rules/src/modifier.rs:147-154`).
   - `LifetimeWire` and sides map onto no kernel type: expiry and teams are
     sim-owned (the kernel keeps no effect store, `tasks/T136.md:343`).
## Tests

New suite `crates/crpg-data/tests/t136b_surface.rs` unless stated; each item
names the Behaviour item(s) it covers.

- **U1 (all).** Every existing `crpg-data` test passes. Struct literals gaining
  fields change only by adding the new fields as `None`/absent
  (`combat_content.rs`, `srd_content.rs`, `round_trip.rs`,
  `tests/support/migration_gate.rs` representatives); T136a's T9 expectation
  becomes the B4 Warning instead of `Vec::new()`. Adding a field to a literal
  does not weaken a test.
- **U2 (B1, B2).** Round trip: an srd-lite ruleset with an `Enum` stat and a
  derived stat, an ability with `bonus_stats`, `Constant` defense and damage
  dice, an effect with `grants_tags` and `Rounds` lifetime, and a participant
  with `side`/`tags`/`initiative_spec` read through `read_document` and
  re-encode to equal `Document`s.
- **U3 (B1, B2).** Absence: every document under `rulesets/minimal-d6/`,
  `rulesets/srd-lite/`, `campaigns/fixtures/combat_basic/` and
  `campaigns/fixtures/combat_srd/` reads with all new optionals `None`; the
  canonical writer emits none of the new keys.
- **U4 (B3).** Each item-3 message is pinned by one failing document
  (`enum_id`, `variants`, depth, nodes, undeclared/self stat, bonus-stats
  shape, opposed dice/attribute, damage dice, granted tags, zero rounds, side,
  participant tags, initiative stat/dice).
- **U5 (B3).** Order: a ruleset with an empty stat name and a bad `derived`
  reports the stat-name fault; an ability with an empty `attribute` and bad
  opposed dice reports the attribute fault (checks appended last).
- **U6 (B4).** Semantic errors: bonus stat outside `attributes`
  (`unknown_stat`); `Bool`-kind bonus stat (`invalid_stat_value`); `ActorStat`
  naming health (`unknown_stat`); undeclared opposed attribute
  (`unknown_stat`); `Stat`-targeted modifier on an undeclared stat and on a
  `Bool` stat (`unknown_stat` / `invalid_stat_value`); prefab missing an
  `Int` stat (`missing_stat`) while a missing `Bool` stat stays clean.
- **U7 (B4).** Warning: `combat_srd` with `recovery_ticks` and
  `repeats_in_real_time` added under its `round_ticks`-less ruleset validates
  to exactly one `Warning` `rt_fields_without_round_ticks` at
  `/recovery_ticks`; with `round_ticks` added the result is `Vec::new()`.
- **U8 (B5).** Migration: `/1` ruleset/ability sources traverse `/1 → /2 →
  /3`; `/1` effect with `duration_rounds: 2` becomes `Rounds { rounds: 2 }`;
  both-present `duration_rounds`+`lifetime` fails with the exact message;
  T136a fields survive every step unchanged (present and absent).
- **U9 (B6).** `generated_schemas()` holds the new properties and none is in
  `required`; `tests/schema_drift.rs` passes against the regenerated files.
- **U10 (B7).** Mapping spot-checks: every `StatKindWire` variant converts
  to its `StatKind`; every `ExprWire` node to its `Expr`; each `DefenseWire`
  variant to its `Against`; `Stat`-targeted modifiers to
  `ModifierTarget::Stat`. (Data-side only; the sim adapter is T137.)

## Goldens and approvals

Needs the user's approval at T136b (frame, `tasks/T136.md:169-172`):
`crates/crpg-data/tests/fixtures/migration_v1/expected.json` ruleset/ability
keys move to `/3`, effect/encounter keys to `/2`; new immediate oracles
`migration_steps/ruleset/1-to-2.json` and `migration_steps/ability/1-to-2.json`
per the T012a convention (`crates/crpg-data/AGENTS.md:234-240`;
`crates/crpg-data/AGENTS.md:279-296`); `migrations.json` gains the four edge
rows sharing the existing `migration_v1/campaign` root (no new fixture root);
22 schemas stay 22 files, four regenerated. Confirmed: **no replay or sim
golden moves** (`tasks/T136.md:369-381`; T137 proves end to end).

## Out of scope

Any code; any other file. T137 (sim adapter) and T138 (RT policy); T134/T135
exposure (`tasks/T136.md:223-225`); `Multiply`/`Clamp` effect ops; `crpg.ui/1`
(T142); hook vocabulary; reaction/interrupt windows.

## Size budget

`src/combat.rs` ≈ +150; `validation.rs` ≈ +60; `document.rs`/`schema.rs`/
`migrations/*` ≈ +80; `tests/t136b_surface.rs` ≈ 350; four schema files
regenerated; docs ≈ +10 each. One session. Stop and report if any file
outside these needs an edit, or if any golden outside §Goldens moves.

## Open questions (each with a recommendation)

1. **Should `Bool`/`Enum`/`Tags`/`Dice` stats need creature values?**
   Recommend no (B4; T137 owns defaults). Alternative: extend
   `Creature.stats` — excluded, that family is unchanged (`tasks/T136.md:35-44`).
2. **How do `Roll`/`Dc` modifier targets bind runtime `RollTag`s?** Recommend
   the sim issues per-ability tags when building `RollRequest`
   (`crates/crpg-rules/src/resolution.rs:281-299`; tags wrap caller-issued
   handles, `crates/crpg-rules/src/modifier.rs:64-69`); data adds no tag
   field. Needs T137's confirmation.
3. **Should derived health be allowed?** Recommend allowing it with no extra
   check; clamping is the sim's policy. Alternative: a read-time ban —
   rejected as gameplay policy in data.
4. **Defer `Multiply`/`Clamp` effect ops?** Recommend yes (D5): the wire
   `value` is `i32`-only, so factors and bounds need new shapes with no
   consumer. Add them with the first consumer.
5. **Opposed payload: `dice` + `attribute` enough?** Recommend yes for the
   planned rulesets; a full nested-roll wire can follow with its first
   consumer. Alternative rejected in D2.
6. **Side-aware end conditions in data?** Recommend no (D6, T137 owns them).

## Agent log

- 2026-10-23 (UTC) · opencode-go/muse-spark-1.3-contributor + T136b draft · Wrote the blind T136b exact-contract draft: additive-first surface with one reshaped lifetime, four version bumps with local-only steps, and the promised RT Warning.
