# T136b-draft-a — blind draft of the T136b exact contract

## Task / Crate(s) / Status

ADR-0029 Follow-on row 3 (`docs/adr/0029-kernel-range-and-time-model.md:482`)
after T136a: typed and derived stats, resolution shape, damage dice, initiative,
effects with tags, and one bump per document with one step each
(`tasks/T136.md:37-45`, `tasks/T136.md:161-173`). Sole crate `crpg-data`, plus
D9's test-literal edits. **S-set.** Status: **proposed draft, not approved.**

## Decision

- **D1. Four tag-only steps; no other family moves.** `crpg.ruleset/2 → /3`,
  `crpg.ability/2 → /3`, `crpg.effect/1 → /2`, `crpg.encounter/1 → /2`;
  `crpg.outcome-table/1` and the rest keep their tags (`tasks/T136.md:39-44`).
  All new fields are optional-missing, so each step rewrites `schema` only and
  every migrated document equals today's value at a new tag
  (`tasks/T136.md:164-167`). *Rejected:* steps that materialise empty vectors or
  `null`s; that moves every migrated byte, so no golden is a re-tag.
- **D2. Typed stats stay externally tagged.** `StatKindWire` gains `Fixed`,
  `Bool`, `Enum`, `Tags`, `Dice` beside `Int`
  (`crates/crpg-rules/src/stats.rs:68-87`); `"kind": "int"` stays byte-identical
  (`rulesets/minimal-d6/ruleset.json:23-40`). *Rejected:* the adjacent
  `type`/`value` shape for all six (`crates/crpg-data/src/combat.rs:60-73`); it
  rewrites every ruleset, so D1 fails.
- **D3. Derived stats live on `StatDecl`.** `derived: Option<ExprWire>` mirrors
  `Expr` (`crates/crpg-rules/src/stats.rs:95-113`), literals mirror
  `SerializableStatValue` (`crates/crpg-rules/src/stats.rs:497-510`). Data cannot
  type or evaluate (`crates/crpg-data/AGENTS.md:159-161`), so it enforces only
  kernel-mirrored bounds and single-document invariants (B6). *Rejected:* a
  name-keyed `Ruleset.derived_stats` list; two surfaces for one stat is the
  second-list drift defect (`crates/crpg-data/AGENTS.md:70-76`).
- **D4. Two defence variants and `bonus_stats`; no threshold field.**
  `DefenseWire::Constant` → `Against::Dc`
  (`crates/crpg-rules/src/resolution.rs:303-312`), `Opposed` →
  `Against::Opposed` (`crates/crpg-rules/src/resolution.rs:323`);
  `bonus_stats` names what the adapter adds to the roll fold
  (`crates/crpg-rules/src/resolution.rs:637-645`). Tiers stay outcome bands: ADR
  K4 composes one `resolve` with one `OutcomeTable::evaluate`
  (`docs/adr/0029-kernel-range-and-time-model.md:100-105`,
  `crates/crpg-rules/src/resolution.rs:189-222`). *Rejected:* a
  `thresholds: Vec<i32>`; a second tier authority with no kernel type.
- **D5. Damage is exactly one of `amount` or `dice` per outcome.** `amount`
  becomes optional-missing and `dice` joins it (`DiceExpr`,
  `crates/crpg-rules/src/dice.rs:45-57`); existing `"amount": n` keeps its
  bytes, so the step stays a re-tag. *Rejected:* a tagged `DamageWire`; it
  rewrites every damage entry and adds a hand-written deserializer.
- **D6. Initiative dice add to the authored constant.**
  `EncounterParticipant.initiative_dice` keeps `initiative`
  (`crates/crpg-data/src/combat.rs:369-377`) as the flat term, so absent is
  today's exact ordering. *Rejected:* replacing `initiative` with an enum; it
  rewrites every encounter and gives ties a second rule.
- **D7. Effects gain a stat target and grants; values stay `i32`.**
  `EffectTargetWire::Stat` → `ModifierTarget::Stat`
  (`crates/crpg-rules/src/modifier.rs:55-62`); `Effect.grants` → `TagSet`
  (`crates/crpg-rules/src/stats.rs:22-23`) read by `ConditionExpr::HasTag`
  (`crates/crpg-rules/src/modifier.rs:96-106`). A stat target must be `Int`:
  `ModOp::Add` takes only `Int`/`Fixed`
  (`crates/crpg-rules/src/modifier.rs:71-89`) and data has no typed value (Q4).
  Tags are not a `Tags`-kinded stat, because `Add` cannot fold a tag set.
  *Rejected:* typed modifier values now, and a `tag_grant` op (inventing kernel
  vocabulary).
- **D8. One new diagnostic code; the rest reuse existing ones.**
  `DiagnosticCode::RealTimeFieldsWithoutRoundTicks` is the `Warning` T136 open
  question 3 asks for (`tasks/T136.md:213-215`); `unknown_stat` covers the new
  references; prefab completeness (`crates/crpg-data/src/validation.rs:702-739`)
  narrows to non-derived `Int`/`Fixed` stats. *Rejected:* three new codes with
  no consumer, and an unchanged completeness rule, which would make every
  `Bool`, `Tags`, `Dice` or derived stat unloadable.
- **D9. Implementation edits struct literals outside `crpg-data`.**
  `StatDecl`, `DamageEntry` and `EncounterParticipant` are literal-built in
  `crates/crpg-sim/tests/{combat,combat_multi,combat_self_kill,history,legal_actions}.rs`,
  `crates/crpg-sim/tests/support/mod.rs` and `crates/crpg-net/tests/events_v2.rs`,
  so T136b is **not** a one-crate diff as `tasks/T136.md:5-6` words it. The edits
  are mechanical and test-only (T017d precedent, `crates/crpg-sim/AGENTS.md`);
  **needs the architect's exception** (Q8). *Rejected:* duplicating the fields on
  `Ability`/`Ruleset` — two authorities for one fact.

## Interface

`crates/crpg-data/src/combat.rs` unless stated; `pub use combat::*` re-exports
them (`crates/crpg-data/src/lib.rs:27`). Attributes and field order are exact;
each field is appended last in its existing struct. Every new item carries a
one-line doc comment (shown for the non-obvious shapes); the elided existing
ones keep theirs.

```rust
use crpg_core::Fx16_16; // new import; core is this crate's only dependency

/// Declared value category of one ruleset stat; mirrors `StatKind`.
/// `Serialize`/`JsonSchema` derived; `Deserialize` hand-written (B2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StatKindWire {
    Int,
    Fixed,
    Bool,
    /// A named domain with its exact variant set, in authored order.
    Enum { enum_id: String, variants: Vec<String> }, // 1..=1024, unique
    Tags,
    Dice,
}

/// One stat literal; mirrors `SerializableStatValue`
/// (`crates/crpg-rules/src/stats.rs:490-510`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", content = "value", rename_all = "snake_case", deny_unknown_fields)]
pub enum StatValueWire {
    Int(i32),
    Fixed(#[schemars(with = "i32")] Fx16_16),
    Bool(bool),
    Enum { enum_id: String, variant: String },
    Tags(Vec<String>),
    Dice(String),
}

/// One derived expression; mirrors `Expr` (`crates/crpg-rules/src/stats.rs:95-113`).
/// `Serialize`/`JsonSchema` derived; `Deserialize` hand-written (B3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum ExprWire {
    Literal(StatValueWire),
    Stat(String), // another stat of this ruleset, by declared name
    Add { left: Box<ExprWire>, right: Box<ExprWire> },
    Subtract { left: Box<ExprWire>, right: Box<ExprWire> },
    Multiply { left: Box<ExprWire>, right: Box<ExprWire> },
    Divide { left: Box<ExprWire>, right: Box<ExprWire> }, // zero divisor = kernel error
    Min { left: Box<ExprWire>, right: Box<ExprWire> },
    Max { left: Box<ExprWire>, right: Box<ExprWire> },
}

// In each struct below the named fields are unchanged and the shown field is
// appended last; Ability appends after T136a's two fields
// (`tasks/T136.md:58-66`).

// StatDecl:
/// Optional derived formula evaluated before this stat's modifiers.
#[serde(default, skip_serializing_if = "Option::is_none")]
pub derived: Option<ExprWire>,

// DefenseWire, two variants appended after `TargetStat`:
/// A constant authored difficulty before its owner's DC modifiers.
Constant {
    /// Any `i32`; the kernel location is `i32`.
    value: i32,
},
/// The target's opposing roll.
Opposed {
    /// Opposing dice notation, 1..=128 UTF-8 bytes.
    dice: String,
},

// DamageEntry (`amount` becomes optional, `dice` is appended):
pub outcome: OutcomeWire,
/// Nonnegative flat damage; present exactly when `dice` is absent.
#[serde(default, skip_serializing_if = "Option::is_none")]
pub amount: Option<u32>,
/// Damage dice notation; present exactly when `amount` is absent.
#[serde(default, skip_serializing_if = "Option::is_none")]
pub dice: Option<String>,

// Ability:
/// Stat names added to this ability's roll, in authored order.
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub bonus_stats: Vec<String>,

// EncounterParticipant:
/// Initiative dice notation added to `initiative`, or absent.
#[serde(default, skip_serializing_if = "Option::is_none")]
pub initiative_dice: Option<String>,

// EffectTargetWire, one variant appended after `Dc`:
/// Contributes to the named stat on the effect's aim.
Stat {
    /// Nonempty stat name declared by the owning ruleset.
    stat: String,
},

// Effect:
/// Tag names granted while this effect is active, in authored order.
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub grants: Vec<String>,
```

Five data-local bound constants (one-line docs), mirroring the kernel:
`MAX_EXPR_NODES` 4096, `MAX_TOTAL_EXPR_NODES` 65_536, `MAX_EXPR_DEPTH` 32,
`MAX_ENUM_VARIANTS` 1024, `MAX_EFFECT_GRANTS` 1024.

Wire forms: `"kind"` is `"int"|"fixed"|"bool"|"tags"|"dice"` or
`{"enum":{"enum_id":"…","variants":["…"]}}`; `derived` is
`{"type":"stat","value":"might"}`, `{"type":"literal","value":{"type":"fixed","value":65536}}`
or `{"type":"add","value":{"left":…,"right":…}}`; defence is
`{"type":"constant","value":{"value":12}}` or `{"type":"opposed","value":{"dice":"2d6"}}`;
a stat target is `{"type":"stat","value":{"stat":"might"}}`.

Elsewhere: `crates/crpg-data/src/migrations/mod.rs` gains `mod effect; mod
encounter;` and four rows (`crates/crpg-data/src/migrations/mod.rs:61-98`);
`.../migrations/ruleset.rs` and `.../migrations/ability.rs` gain `v2_to_v3`; new
`.../migrations/effect.rs` and `.../migrations/encounter.rs` each hold one
`v1_to_v2`. Tags become `crpg.ruleset/3`
(`crates/crpg-data/src/document.rs:432`), `crpg.ability/3`
(`crates/crpg-data/src/document.rs:435`), `crpg.encounter/2`
(`crates/crpg-data/src/document.rs:441`), `crpg.effect/2`
(`crates/crpg-data/src/document.rs:444`); `crates/crpg-data/src/schema.rs:116-124`
mirrors them. `crates/crpg-data/src/validation.rs:100` gains one
`DiagnosticCode` variant after `UnknownPool`. The registry stays 20 families and
`generated_schemas` 22 files (`crates/crpg-data/src/schema.rs:90-151`);
`Document` gains no variant.

## Migration steps

Each step is a private `fn(&mut Value)` shaped like the existing ones
(`crates/crpg-data/src/migrations/ruleset.rs:14-35`), rejects only a source-tag
mismatch, and writes nothing but `schema`.

- **ruleset `2 → 3`** (`crates/crpg-data/src/migrations/ruleset.rs::v2_to_v3`).
  A non-`/2` source fails `Malformed`
  `ruleset migration expects schema crpg.ruleset/2`. Keeps byte for byte: `id`,
  `slug`, `name`, `_note`, `package`, `version`, `stats` (all `kind` strings and
  any `derived`), `health_stat`, `attributes`, `pools`, `abilities`, and T136a's
  `round_ticks`, present or absent. Adds no key; tag → `crpg.ruleset/3`. No key
  is rejected: a `/2` source carrying `derived` or a `/3` `kind` migrates and
  decodes (T136 open question 5 permits rejection; we accept, as the `1 → 2`
  steps do, `tasks/T136.md:219-221`).
- **ability `2 → 3`** (`crates/crpg-data/src/migrations/ability.rs::v2_to_v3`).
  A non-`/2` source fails `ability migration expects schema crpg.ability/2`.
  Keeps `dice`, `attribute`, `outcome_table`, `damage` (each `amount`), `cost`,
  `extra_costs`, `ends_turn`, `effect`, `defense` (all four variants),
  `natural_die`, `requires_target`, `allow_self_target`, and T136a's
  `recovery_ticks` and `repeats_in_real_time` unchanged. Adds no `bonus_stats`
  and no `dice`. Tag → `crpg.ability/3`.
- **effect `1 → 2`** (`crates/crpg-data/src/migrations/effect.rs::v1_to_v2`).
  A non-`/1` source fails `effect migration expects schema crpg.effect/1`. Keeps
  `id`, `slug`, `name`, `_note`, `aim`, `mod_type`, `policy`, every `modifiers`
  entry (`target`, `op`, `value`, `priority`, `name`) and `duration_rounds`.
  Adds no `grants`. Tag → `crpg.effect/2`.
- **encounter `1 → 2`** (`crates/crpg-data/src/migrations/encounter.rs::v1_to_v2`).
  A non-`/1` source fails
  `encounter migration expects schema crpg.encounter/1`. Keeps `id`, `slug`,
  `name`, `_note`, `ruleset`, and each participant's `placement` and
  `initiative` in authored order. Adds no `initiative_dice`. Tag →
  `crpg.encounter/2`.

`rulesets/minimal-d6/` (ruleset and ability `/1`) and `rulesets/srd-lite/`
(ruleset and abilities `/2`, effect `/1`) stay **byte-identical** and migrate in
memory through the whole chain before typed decode
(`crates/crpg-data/AGENTS.md:43-50`), so `1 → 2 → 3` and `2 → 3` end at today's
runtime values plus a tag; likewise `campaigns/fixtures/combat_basic/`
(encounter `/1`) and `campaigns/fixtures/combat_srd/`. `crpg.outcome-table/1` is
untouched, so `rulesets/minimal-d6/roll_under.json:19` and
`rulesets/srd-lite/heavy_table.json:33` are unchanged.

## Behaviour

1. **Decode: tags.** `read_document` accepts the four new tags; `/1` and `/2`
   sources migrate through the registered chain first
   (`crates/crpg-data/src/migrations/mod.rs:288-310`). Future, unknown and
   non-canonical tags stay `UnsupportedSchema`.
2. **Decode: `StatKindWire`.** A `kind` is exactly one of `"int"`, `"fixed"`,
   `"bool"`, `"tags"`, `"dice"`, or exactly
   `{"enum":{"enum_id":<string>,"variants":[<string>,…]}}` with no other key at
   either level. Another string is an unknown-variant failure; an extra or
   missing key fails; `{"type":"int"}` fails.
3. **Decode: `derived`.** Absent or explicit `null` reads `None`
   (`tasks/T136.md:78-79`); present must be one `ExprWire` with exactly `type`
   and `value`, where `value` is a `StatValueWire` object (`literal`), a JSON
   string (`stat`), or an object with exactly `left` and `right` (the six
   binaries). `StatValueWire` is adjacently tagged, rejects unknown keys, stores
   `fixed` as an `i32` raw integer, `tags` as strings, `dice` as a string.
4. **Decode: other shapes.** `amount` (integer `0..=u32::MAX`), `dice`,
   `initiative_dice`, `bonus_stats` and `grants` entries are optional-missing;
   `null` is absent for the three scalars, a non-array fails for the vectors,
   entries must be strings. `DefenseWire` and `EffectTargetWire` accept exactly
   the forms above. Unknown fields stay rejected.
5. **Encode.** Absent fields write no key; empty `grants`/`bonus_stats` write no
   key; `amount: Some(0)` writes `"amount":0`; `variants` and `grants` keep
   authored order. The writer sorts keys and adds one LF
   (`crates/crpg-data/src/canonical.rs:74-80`); every existing document's bytes
   are unchanged except its `schema` value.
6. **Read-time: ruleset** (appended after T136a's `round_ticks` check,
   `crates/crpg-data/src/combat.rs:707-766`), per stat then per expression in
   authored order: `ruleset enum stat domain must be nonempty`;
   `ruleset enum stat variants must be 1..=1024`;
   `ruleset enum stat variants must be nonempty and unique`;
   `ruleset derived expression must be <=4096 nodes`;
   `ruleset derived expression depth must be <=32`;
   `ruleset derived stat references must name declared stats`;
   `ruleset derived expressions must total <=65536 nodes`;
   `ruleset derived stats must be acyclic` (first stat on a cycle, last).
7. **Read-time: ability** (appended after T136a's `recovery_ticks` check,
   `crates/crpg-data/src/combat.rs:767-810`):
   `ability bonus stat names must be nonempty`; `duplicate ability bonus stat`;
   `ability damage entry must carry exactly one of amount or dice`;
   `ability damage dice must be 1..=128 UTF-8 bytes`;
   `ability defense dice must be 1..=128 UTF-8 bytes`;
   `ability opposed defense requires a target` (when `requires_target` is
   false). Dice bounds are byte lengths, mirroring `MAX_DICE_INPUT_BYTES`
   (`crates/crpg-data/src/combat.rs:30-31`); no notation is parsed here.
8. **Read-time: encounter and effect** (`crates/crpg-data/src/combat.rs:841-876`):
   `encounter initiative dice must be 1..=128 UTF-8 bytes`;
   `effect modifier stat must be nonempty` (inside the modifier loop, after the
   duplicate-id check); `effect grants must be <=1024`;
   `effect grant tags must be nonempty and unique`.
9. **Read-time order.** Each new check follows its arm's existing checks and
   T136a's, so a multi-fault document reports the earlier fault as today; the
   new checks run in items 6–8's order.
10. **Semantic: `unknown_stat`.** `walk_combat`
    (`crates/crpg-data/src/validation.rs:534-742`) reports as `Error`:
    `/bonus_stats/<i>` when the name is not a declared non-health `Int` stat of
    the owning ruleset (`"unknown bonus stat {:?} for ability: not a declared
    non-health int stat of ruleset {}"`), and `/modifiers/<i>/target` for an
    effect `Stat` target when no ruleset listing an ability that references the
    effect declares it (`"unknown stat {:?} for effect {}: not a declared
    non-health int stat of ruleset {}"`). One diagnostic per reference; the
    smallest referencing ruleset id is named; orphans skip.
11. **Semantic: prefab completeness narrowed.** `missing_stat` and the
    whole-number/positive-health `invalid_stat_value` checks apply only to
    stats with no `derived` and kind `Int` or `Fixed`; codes, pointers and
    messages for existing fixtures are unchanged.
12. **Semantic: the RT `Warning`.** `RealTimeFieldsWithoutRoundTicks`,
    `Warning`, at `/recovery_ticks` if present else `/repeats_in_real_time`, for
    an ability of a ruleset whose `round_ticks` is absent: `"ability {}
    declares real-time fields but ruleset {} declares no round_ticks"`, fix
    `"add round_ticks to the ruleset or remove the real-time fields"`. Orphans
    skip; an unused `round_ticks` stays clean; warnings never fail validation
    (`crates/crpg-data/src/validation.rs:25-33`).
13. **Kernel mapping.** Every shape names an existing kernel type; no kernel
    change is assumed. Missing kernel types are Q1, Q2, Q4, Q6 and Q9.

    | Data shape | Kernel type |
    |---|---|
    | `StatKindWire` | `StatKind` (`crates/crpg-rules/src/stats.rs:68-87`); `Enum.variants` → `BTreeSet<String>` (`crates/crpg-rules/src/stats.rs:76-82`) |
    | `StatValueWire` | `StatValue` (`crates/crpg-rules/src/stats.rs:50-65`); `Enum` → `EnumValue` (`crates/crpg-rules/src/stats.rs:30-37`, `crates/crpg-rules/src/stats.rs:59`) |
    | `ExprWire` | `Expr`; `Stat(String)` → `Expr::Stat(StatId)` (`crates/crpg-rules/src/stats.rs:95-113`) |
    | `StatDecl.derived` | `StatDefinition.derived: Option<Expr>` (`crates/crpg-rules/src/stats.rs:115-124`) |
    | `Ability.bonus_stats[i]` | `Modifier{target: ModifierTarget::Roll(RollTag), op: ModOp::Add(…)}` (`crates/crpg-rules/src/modifier.rs:53-89`, `crates/crpg-rules/src/modifier.rs:108-127`), folded at `crates/crpg-rules/src/resolution.rs:637-645` |
    | `DefenseWire::ActorAttribute` / `TargetStat` | `Against::Stat{entity: actor / target}` (`crates/crpg-rules/src/resolution.rs:313-321`) |
    | `DefenseWire::Constant` | `Against::Dc{value, owner: actor}` (`crates/crpg-rules/src/resolution.rs:303-312`) |
    | `DefenseWire::Opposed` | `Against::Opposed(RollRequest)` + `RollSpec::Dice` (`crates/crpg-rules/src/resolution.rs:270-299`, `crates/crpg-rules/src/resolution.rs:323`) |
    | `DamageEntry.dice` | `DiceExpr` (`crates/crpg-rules/src/dice.rs:45-57`) + `evaluate` (`crates/crpg-rules/src/dice.rs:130-170`) |
    | `EncounterParticipant.initiative_dice` | `DiceExpr` total plus `initiative`, saturating add |
    | `EffectTargetWire::Stat` | `ModifierTarget::Stat(StatId)` (`crates/crpg-rules/src/modifier.rs:55-58`) |
    | `EffectTargetWire::{Roll,Dc}` | `ModifierTarget::{Roll,Dc}(RollTag)` (`crates/crpg-rules/src/modifier.rs:59-62`) |
    | `Effect.grants[i]` | `TagSet = BTreeSet<TagId>` (`crates/crpg-rules/src/stats.rs:22-23`) via `ConditionExpr::HasTag` (`crates/crpg-rules/src/modifier.rs:96-106`) |
    | `Effect.mod_type` / `policy` | `ModTypeId` / `StackingPolicy`, unchanged (`crates/crpg-rules/src/modifier.rs:33-35`, `crates/crpg-rules/src/modifier.rs:129-139`) |

14. **Invariants unchanged.** 20 families, 22 schema files, the same layout
    predicates and canonical writer, no dependency change
    (`crates/crpg-data/AGENTS.md:96-105`).

## Tests

New suite `crates/crpg-data/tests/surface_v3.rs` unless stated.

- **T1 (B1).** `tags_and_historical_chains`: the four tags read; `/1`/`/2` reach
  the same values; `/4`, `/4`, `/3`, `/3` fail `UnsupportedSchema`.
- **T2 (B2).** `stat_kinds_all_six_forms`: six forms round-trip; rejected
  spellings fail `Malformed`; `"kind":"int"` bytes from
  `rulesets/minimal-d6/ruleset.json`.
- **T3 (B3).** `derived_expressions_mirror_the_kernel_tree`: all eight `ExprWire`
  and six `StatValueWire` arms re-encode exactly; `null` is absent; rejected
  shapes fail.
- **T4 (B4).** `new_fields_are_optional_missing`: the absent/null/typed matrix.
- **T5 (B5).** `canonical_output_omits_absent_and_keeps_authored_order`: exact
  strings including `"amount":0` and an empty `grants`.
- **T6 (B6).** `ruleset_read_time_messages_and_bounds`: eight messages, each
  bound and one past (4096/4097, 32/33, 1024/1025), a reference failure, a cycle.
- **T7 (B7).** `ability_read_time_messages`: each message, dice bytes 128/129,
  non-targeted `opposed`.
- **T8 (B8).** `encounter_and_effect_read_time_messages`: each message, grants
  1024/1025.
- **T9 (B9).** `earlier_faults_still_win`: empty `attribute` with
  `bonus_stats: [""]`; duplicate damage outcome with empty `dice`; empty
  `mod_type` with an empty stat target.
- **T10 (B10).** `unknown_bonus_and_effect_stats`: unknown, health and `Fixed`
  targets; one diagnostic per reference; orphans clean; smallest ruleset named.
- **T11 (B11).** `prefab_completeness_skips_derived_and_non_numeric`: a missing
  plain `Int` stat still reports `missing_stat`; `Bool`/`Tags`/derived do not.
- **T12 (B12).** `real_time_warning_only_under_tb_only_rulesets`: the Warning at
  both pointers; `validate` succeeds; used and unused `round_ticks` are clean.
- **T13 (B13).** `wire_fixtures_pin_every_mapping_row`: one exact JSON literal
  per row of item 13.
- **T14 (B14).** `tests/schema_drift.rs` stays byte-comparing; its pins become
  `/3`, `/3`, `/2` and an explicit `/2` encounter arm
  (`crates/crpg-data/tests/schema_drift.rs:181-191`); `round_trip` and
  `canonical` pass byte-unchanged; `surface_v3` asserts 20 families and 22 files.
- **T15 (D1, steps).** `tests/migrations.rs::steps_are_tag_only`: each step
  differs only in `schema`, a wrong source tag fails with the exact message,
  T136a's fields survive; `crates/crpg-data/tests/migrations.rs:135-143` → `/3`.
- **T16 (D1, registry).** `tests/migrations.rs` and
  `crates/crpg-data/src/migrations/mod.rs:347-396` assert 20 families, `current`
  3/3/2/2 and 1 for the rest.
- **T17 (goldens).** `tests/migration_coverage.rs` passes against the updated
  manifest, moved step oracles and campaign golden
  (`crates/crpg-data/tests/support/migration_gate.rs:263-312`).
- **T18 (existing suites).** `crates/crpg-data/tests/combat_content.rs:127-135`,
  `crates/crpg-data/tests/srd_content.rs:425-455`,
  `crates/crpg-data/tests/pointer_edit.rs:618` and `tests/schema_drift.rs`
  change only their pinned values; `tests/rt_fields.rs` passes at the new tags;
  the D9 literals gain only the new fields. Nothing is weakened or deleted.

## Goldens and approvals

Every change needs the user's approval at T136b (`tasks/T136.md:168-172`).

- `crates/crpg-data/tests/fixtures/migration_v1/expected.json`:
  `rulesets/migrated.json` and `abilities/migrated.json` become today's values
  with only `schema` changed (`/3`); the `creatures/creature.json` key gains
  `hp` and `pow` (whole; `hp` positive) so the new encounter passes item 11;
  two keys are added — `effects/migrated.json` and `encounters/migrated.json`
  at `/2`. Every other key is byte-identical.
- New documents under `migration_v1/campaign/`: `effects/migrated.json`
  (`crpg.effect/1`, id `…001Y`, aimed `slf`, `stack_all`, one `roll` modifier,
  `duration_rounds: 1`) and `encounters/migrated.json` (`crpg.encounter/1`, id
  `…001V`, ruleset `…001W`, one participant: placement `…0005`, `initiative:
  10`). Both validate cleanly so the root stays clean
  (`crates/crpg-data/AGENTS.md:84-94`).
- New `crates/crpg-data/tests/fixtures/migration_steps/crpg.ruleset/1-to-2.json`
  and `.../crpg.ability/1-to-2.json`: today's `/2` values moved verbatim out of
  the campaign golden. No new root.
- `crates/crpg-data/tests/fixtures/migrations.json` gains four rows sorted by
  `(schema_type, from)`: ability `2→3`, encounter `1→2`, effect `1→2`, ruleset
  `2→3`, each rooted at `migration_v1/campaign` with that golden; the existing
  three rows are unchanged.
- `schemas/{ruleset,ability,effect,encounter}.schema.json` are regenerated by
  the existing example (`crates/crpg-data/AGENTS.md:114`); the other 18 are
  byte-identical. `crates/crpg-data/tests/fixtures/expected.json` stays
  unchanged: three roots, `migration_v1/campaign` still clean.
- **No replay or sim golden moves.** `crates/crpg-testkit/goldens/*`,
  `crates/crpg-sim/tests/goldens/history_v1.*`, every `state_hash` and
  `history_hash` stay byte-identical; `rulesets/minimal-d6/`,
  `rulesets/srd-lite/`, `campaigns/fixtures/` and
  `crates/crpg-sim/tests/combat.rs:1526`, `crates/crpg-sim/tests/combat.rs:1530`
  are untouched. A golden that moves is a stop, not a re-bless.

## Out of scope

Any code; any other draft. T137 (adapter), T138 (RT policy), T139 (persistent
creature state), T134/T135 exposure (T136 open question 6,
`tasks/T136.md:222-225`), `crpg.ui/1` (T142), every kernel change, and the sim's
runtime behaviour for any new field.

## Size budget

`src/combat.rs` ≈ +230/−20; `src/migrations/*` ≈ +120; `src/validation.rs` ≈
+90; `tests/surface_v3.rs` ≈ 550; `tests/migrations.rs` and
`tests/support/migration_gate.rs` ≈ +120; fixtures and goldens ≈ +260; docs ≈
+20 each (`docs/architecture/crpg-data.md`, `crates/crpg-data/AGENTS.md`,
signed); D9's literals ≈ 20 lines in ~9 files. Two sessions, not one.

## Open questions (each with a recommendation)

1. **Sides.** No kernel type (`crates/crpg-data/src/combat.rs:369-377`,
   `docs/adr/0029-kernel-range-and-time-model.md:38-40`). Recommend deferring to
   T137/T139; a `side` label with no reader is the vocabulary-without-a-consumer
   trap (`crates/crpg-sim/AGENTS.md`).
2. **Effect lifetime kinds.** No kernel type; effects are sim-owned
   (`docs/adr/0029-kernel-range-and-time-model.md:41-43`). Recommend
   `duration_rounds` stays the only lifetime.
3. **Stat-derived initiative.** Recommend rolled dice plus the constant (D6); an
   `initiative_stats` list can follow its consumer.
4. **Typed modifier values.** `EffectModifierWire.value: i32`
   (`crates/crpg-data/src/combat.rs:180`) cannot set a `Fixed`, `Bool`, `Enum`,
   `Tags` or `Dice` stat. Recommend `Int`-only stat targets (item 10) and a
   typed value with its first consumer.
5. **Non-`Int` creature values.** `Creature.stats` holds raw integers
   (`crates/crpg-data/src/document.rs:117-119`), so non-numeric kinds have no
   authored value. Recommend item 11's narrowing and T139 for the value shape.
6. **Tiered thresholds.** Recommend no field: outcome bands already express
   tiers on one margin (D4, ADR K4).
7. **`Constant` owner.** Recommend the actor, so no target is needed and it
   matches `actor_attribute`; the target's fold needs `requires_target` and a
   second read-time rule.
8. **D9, the one-crate rule.** Recommend an explicit test-only exception for the
   nine downstream files; the alternative duplicates fields on
   `Ability`/`Ruleset`, which the crate forbids.
9. **Crit doubling** (`docs/adr/0029-kernel-range-and-time-model.md:125`). No
   kernel type multiplies a dice result; recommend a later row.
10. **Steps accept later keys.** Recommend accepting them, matching `1 → 2`.

## Agent log

- 2026-10-23 (UTC) · opencode-go/deepseek-v4.1-flash + T136b draft · Wrote the blind T136b contract draft: four tag-only version bumps, typed and derived stats, damage dice, initiative, effects with tags and stat targets, the kernel mapping table, and the migration-fixture and downstream-literal consequences, with every kernel shortfall raised as an open question instead of invented.
