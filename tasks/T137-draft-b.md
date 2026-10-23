# T137 — `crpg-sim` adapter generalization (ADR-0029 Follow-on 4)

## Task / Crate(s) / Status
The exact contract for ADR-0029 Follow-on 4 (`docs/adr/0029-kernel-range-and-time-model.md:483`): the combat adapter executes the authored surface T136b adds to `crpg-data` (`tasks/T136b.md:80-235`, `:289-428`) through the `crpg-rules` kernel, with every existing replay, history and hash golden byte-identical. It deletes the T136c `UnsupportedAuthored` bridge (`tasks/T136b.md:430-448`, `:460-461`) and extends `ability_is_legacy` (T138 hand-off, `tasks/T138.md:846-850`).

**Crate: `crpg-sim` only. S-set (the `World` struct and its hash). Status: proposed draft.** Two implementation packages, T137a/T137b, plus the T137c golden (Size budget).

**Base:** T136b and its batch companion T136c, and T138a. T138a's hand-off binds this contract: T137 builds on `start_encounter_tracked(world, spec, mode)` (`tasks/T138.md:846-848`), RT keys stay clock-based, and an initiative spec changes only TB keys and the RT→TB rebuild (`:848-850`). If T138a is not yet merged, the two RT terms (mode parameter, `recovery_ticks`/`repeats_in_real_time` in `ability_is_legacy`) are dropped mechanically; nothing else changes.

**Companion package in another crate: none.** The one candidate, T134 (`crpg-rules`, `docs/adr/0029-kernel-range-and-time-model.md:480`), is deliberately not written now (D1). Adding one `CombatError` variant and deleting another is a `crpg-sim` edit under ADR-0029, not a second crate.

Nothing here is approved. A Claude architect adjudicates it against another draft; the user approves.

## Decision
- **D1. T134 is not needed for T137, and is not written now.** Every T136b dice field is an ordinary `DiceExpr` sum: `Ruleset.initiative.dice` (`tasks/T136b.md:177-182`), `DefenseWire::Opposed.dice` (`:199-204`), `DamageEntry.dice` (`:209-213`), `StatValueWire::Dice` (`:143-145`). No T136b shape selects a success-count or face-count mode, and `DiceExpr` has no mode field to set (`crates/crpg-rules/src/dice.rs:51-57`). A kernel mode with no authored selector is vocabulary without a consumer (`crates/crpg-sim/AGENTS.md:295-297`). *Rejected:* land T134 first because the ADR row pairs it with T137 — it would add a second crate to the batch for a mode nothing can request. T134 returns with the first data shape that selects counting.
- **D2. Typed stats are one sim-owned symbolic declaration vector on `CombatDefinition`, and bases split by kind.** `CombatDefinition.stats: Vec<StatDeclaration>` (last field) is populated only when the encounter needs the kernel stat surface (B3), so legacy worlds write no key. `Int` bases stay in `Combatant.attributes` (`crates/crpg-sim/src/combat.rs:77`); `Fixed` bases go in a new last `Combatant.fixed_stats`. *Rejected:* holding `crpg_data::StatDecl`/`ExprWire` in hashed sim state — it couples sim save bytes to the data schema and breaks the mirror convention every other adapter enum follows (`AbilityDefense` `crates/crpg-sim/src/combat.rs:140-148`, `EffectTarget` `:163-168`); replacing `attributes` with one typed vector — it moves every legacy `Combatant` byte.
- **D3. Derived formulas are a sim-owned symbolic AST, rebuilt into a kernel `ModifierPipeline` per action, with stat handles looked up and never interned on the execution path.** *Rejected:* persisting `crpg_rules::Expr` — it holds `StatId` and has no serde (`crates/crpg-rules/src/stats.rs:95-113`), so it would reintroduce the E014 renumbering bug; persisting evaluated derived values — they go stale the moment a `ModifierTarget::Stat` effect attaches (`crates/crpg-rules/src/modifier.rs:54-62`).
- **D4. `Int` and `Fixed` declarations take a base from `Creature.stats`; any other non-derived kind is rejected at start as `InvalidStatValue`.** `Creature.stats` is `BTreeMap<String, Fx16_16>` (`crates/crpg-data/src/document.rs:117-119`), and T136b changes no creature shape (`tasks/T136b.md:582-583`), so a `Bool`/`Enum`/`Tags`/`Dice` base has no authored source; such a stat must be `derived` (a literal root is expressible, `tasks/T136b.md:150-172`). T136b's `missing_stat` narrowing to non-derived `Int`/`Fixed` (`:390-393`) makes the document data-clean; it says nothing about a base for the other kinds. *Rejected:* inventing defaults (invents rules the ADR does not grant); silently dropping the declaration (drops authored data and hides the authoring trap).
- **D5. The health stat is a base entry in the generalized block.** T136b lets a derived formula reference any declared stat (`tasks/T136b.md:340-341`), so `Int(combatant.health())` is inserted for `health_stat` and data-legal `power = health / 2` works. *Rejected:* excluding health from the block — a data-clean formula would fail at start.
- **D6. Defence selection is one `match` per action producing `Against` over the existing views.** `Constant`→`Against::Dc{value, owner: target}` exactly as `ActorAttribute` today (`crates/crpg-sim/src/combat.rs:1885-1890`; mapping `tasks/T136b.md:422`), `ActorStat`→`Against::Stat{entity: actor}` (`tasks/T136b.md:423`), `Opposed`→`Against::Opposed(RollRequest{actor: target, …})` (`tasks/T136b.md:424`; kernel `crates/crpg-rules/src/resolution.rs:302-324`). *Rejected:* a second resolve call per variant — the kernel already selects the side from `Against`.
- **D7. `bonus_stats` become synthetic `ModifierTarget::Roll` `Add` modifiers under one reserved modifier type with `StackAll`.** Ids are the lowest unused `Ulid`s of that view (starting at `Ulid::from_u128(1)`, `crates/crpg-core/src/ulid.rs:32`), `priority: 0`, `condition: None`, `source.kind = "bonus_stat"`, `source.id` = the ability id. A referenced effect whose `mod_type` is exactly `combat.bonus_stat` is rejected at start as `InvalidEffect`, so the forced policy can never silently restack authored content. *Rejected:* a new kernel "add to roll" API (a `crpg-rules` change, out of scope); reusing an authored `mod_type` (its policy could suppress a contribution under `HighestBonusWorstPenalty`, `crates/crpg-rules/src/modifier.rs:129-139`).
- **D8. Damage dice are a new `AbilityDefinition.damage_dice` vector beside the flat map, drawn after `resolve`, clamped once.** Changing `damage: Vec<(Outcome, u32)>` (`crates/crpg-sim/src/combat.rs:239`) would move every legacy definition byte. The roll happens after the resolution commit (`crates/crpg-rules/src/resolution.rs:612-645`) and before `spend_costs` (`crates/crpg-sim/src/combat.rs:1988`), so it is the next draw on `combat.roll` and a failed resolution still leaves the RNG untouched. *Rejected:* rolling before or inside resolution, which would make a rejected action draw; widening the flat tuple type, which changes the persisted wire for every legacy ability.
- **D9. Initiative is rolled once at start in both modes, in authored participant order, and stored.** `Combatant.initiative` (`crates/crpg-sim/src/combat.rs:76`) holds the rolled TB key; TB schedules at it (`crates/crpg-sim/src/combat.rs:1400`), RT keeps key `0` (`tasks/T138.md:495-501`), and TB rollover (`crates/crpg-sim/src/combat.rs:2181-2189`) and the RT→TB rebuild (`tasks/T138.md:573-580`) reuse the stored value — the hand-off's "stored initiative" (`tasks/T138.md:848-850`). *Rejected:* re-rolling per round — rollover is a container re-add, and re-rolling would add a draw inside a branch that currently draws nothing; rolling only in TB — an RT→TB rebuild would need a late, mode-dependent draw.
- **D10. One modifier collection per combatant per action (`collect_modifiers`), carrying all three target kinds.** Roll/DC folds filter by target (`crates/crpg-rules/src/modifier.rs:610-615`; `:1252-1277`), so the same slice serves every fold and the self-targeted union (`crates/crpg-sim/src/combat.rs:1931-1944`) disappears. *Rejected:* keeping `roll_modifiers`/`dc_modifiers` and adding a third collector — three lists that must agree, and `check_context` would validate each separately (`crates/crpg-rules/src/modifier.rs:637-668`).
- **D11. `grants` are projected into the holder's `TagSet` on every fold; placement tags persist only when a grant exists.** Populate `Combatant.tags` and `EffectDefinition.grants` only when some referenced effect has grants; otherwise srd-lite's combatants (which all carry `Creature.tags`, `campaigns/fixtures/combat_srd/creatures/hero.json:15`) would gain a key and move the srd golden. *Rejected:* always storing placement tags (moves bytes for content that cannot read them); deferring grants to T139 (drops a T136b shape the ADR row 4 lists).
- **D12. One new `CombatError::TooManyTags`; `UnsupportedAuthored` is deleted.** The projected tag set (placement tags plus every granted tag) can exceed `MAX_TAGS` 1024 even though each grant list is bounded (`tasks/T136b.md:366-367`), and no existing variant is honest about it. Fieldless, `Display` = `TooManyTags at spec/tags`, last variant; `UnsupportedAuthored` (`tasks/T136b.md:431-436`) and its arm disappear. *Rejected:* reusing `InvalidEffect` (blames one effect) or `ValueOverflow` (a defensive conversion overflow, `crates/crpg-sim/src/combat.rs:746-748`, whose location is wrong).
- **D13. Legacy preservation is by construction, not by re-bless.** Every new field is last and skipped at its legacy value, `CombatDefinition::serialize`'s 15/18 branch uses an extended `definition_is_legacy`, and a legacy-shaped encounter interns nothing new and draws nothing new (Hash and goldens). *Rejected:* regenerating goldens after the fact (ADR-0012/0029 forbid it: `docs/adr/0029-kernel-range-and-time-model.md:425-427`).
- **D14. Load coherence extends `World` deserialization with the exact messages in B17.** A save must never reach an action-time `expect` (`crates/crpg-sim/AGENTS.md:347-351`; the existing check is `crates/crpg-sim/src/world.rs:149-522`). *Rejected:* validating lazily in `perform_action`.
- **D15. Kernel failures at start map onto the existing vocabulary; only `TooManyTags` is new.** A bad value or formula is `InvalidStatValue { stat }` (`crates/crpg-sim/src/combat.rs:729-734`), an unparseable notation is `InvalidDice` (`:742-743`), an undeclared or wrong-kind name is `MissingStat` (`:723-728`), and a reserved modifier type is `InvalidEffect` (`:763-768`). *Rejected:* one new variant per kernel code (DivisionByZero, TypeMismatch, …) — it exports the kernel's error taxonomy into the sim's public API for faults data already validates.

## Interface
Every new item is public unless marked private, has a doc comment (`#![warn(missing_docs)]`), and is re-exported from `src/lib.rs` (`crates/crpg-sim/src/lib.rs:33-40`). No dependency, no float outside `Transform`, no `HashMap`/`HashSet`, no `unsafe`. New fields are the **last** field of their struct; new variants come after the existing ones, in order.

### `crates/crpg-sim/src/combat.rs`
Imports gain `DeterministicRng`, `Interners`, `TagId` and `Fx16_16` from `crpg_core`, and `MAX_EXPR_DEPTH`, `MAX_EXPR_NODES`, `MAX_TAGS` from `crpg_rules`. Placed after `COMBAT_ROLL_TAG` (`:57`):
```rust
/// Reserved modifier type for bonus-stat roll contributions (T137).
const BONUS_STAT_MOD_TYPE: &str = "combat.bonus_stat";

/// Declared value category of one persisted combat stat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatKindDecl {
    /// Whole numbers.
    Int,
    /// Fixed-point numbers.
    Fixed,
    /// Flags.
    Bool,
    /// A named domain with its exact variant list.
    Enum { enum_id: String, variants: Vec<String> },
    /// Tag sets.
    Tags,
    /// Dice-expression values.
    Dice,
}

/// One typed literal inside a derived formula.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "value")]
pub enum StatLiteral {
    Int(i32),
    Fixed(Fx16_16),
    Bool(bool),
    Enum { enum_id: String, variant: String },
    Tags(Vec<String>),
    Dice(DiceExpr),
}

/// One symbolic derived-stat formula, mirrored from the kernel expression tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "value")]
pub enum StatFormula {
    Literal(StatLiteral),
    Stat(String),
    Add { left: Box<StatFormula>, right: Box<StatFormula> },
    Subtract { left: Box<StatFormula>, right: Box<StatFormula> },
    Multiply { left: Box<StatFormula>, right: Box<StatFormula> },
    Divide { left: Box<StatFormula>, right: Box<StatFormula> },
    Min { left: Box<StatFormula>, right: Box<StatFormula> },
    Max { left: Box<StatFormula>, right: Box<StatFormula> },
}

/// One declared stat and its optional formula, in authored stat order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatDeclaration {
    pub name: String,
    pub kind: StatKindDecl,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derived: Option<StatFormula>,
}

/// One rolled-initiative rule: dice plus added non-health int stats.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InitiativeRule {
    pub dice: DiceExpr,
    pub bonus_stats: Vec<String>,
}
```
`Combatant` (`:73-88`) gains two last fields and two accessors:
```rust
    /// Fixed-point bases of declared non-int stats, authored order; skipped when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    fixed_stats: Vec<(String, Fx16_16)>,
    /// Authored placement tags; present only when an effect grants tags.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
// impl Combatant, after `attached` (:131-134):
    /// Fixed-point bases of declared non-int stats in authored order.
    pub fn fixed_stats(&self) -> &[(String, Fx16_16)];
    /// Authored placement tag names.
    pub fn tags(&self) -> &[String];
```
`AbilityDefense` (`:137-148`) gains three last variants:
```rust
    /// Uses a constant defence value; the target's DC modifiers apply.
    Constant(i32),
    /// Uses the actor's named non-health int stat.
    ActorStat { stat: String },
    /// Uses the target's opposing roll total.
    Opposed { dice: DiceExpr, bonus_stats: Vec<String> },
```
`EffectTarget` (`:160-168`) gains last `Stat { stat: String }`.
`AbilityDefinition` (`:225-252`) gains two last fields:
```rust
    /// Non-health int stats added to this ability's roll; skipped when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bonus_stats: Vec<String>,
    /// Optional damage dice per outcome label; skipped when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub damage_dice: Vec<(Outcome, DiceExpr)>,
```
`EffectDefinition` (`:197-212`) gains last `grants: Vec<String>` (`#[serde(default, skip_serializing_if = "Vec::is_empty")]`), and its hand-written pair (`:340-389`) gains the field: serialize 7 fields with `grants` last, `EffectDefinitionRepr` gains `#[serde(default)] grants`.
`CombatDefinition` (`:262-300`) gains two last fields:
```rust
    /// Declared stats in authored order; empty for the legacy int-only surface.
    pub stats: Vec<StatDeclaration>,
    /// Rolled initiative; absent means each participant's authored key alone.
    pub initiative: Option<InitiativeRule>,
```
`serialize` (`:391-425`) keeps 15/18 and appends `stats` and `initiative` inside the non-legacy branch (both skipped when empty); `CombatDefinitionRepr` (`:427-451`) gains `#[serde(default)] stats: Vec<StatDeclaration>` and `initiative: Option<InitiativeRule>`; the legacy rebuild (`:466-479`) sets `stats: Vec::new()`/`initiative: None`; the vector branch passes the decoded values. `definition_is_legacy` is `vectors_are_legacy(definition) && definition.stats.is_empty() && definition.initiative.is_none()`, so the 15-vs-18 choice is today's choice whenever the two new fields are absent. `attribute_names`'s doc (`:276-277`) narrows to "declared non-health int stat names in authored stat order"; its value is unchanged for every legacy world (all their non-health declarations are `Int`).

**Action-time order** (pinned; the legacy surface keeps today's exact code path): `validate_action` unchanged (`:1804`) → ability entry and `roll_tag` (`:1836-1843`) → policies from `definition.effects` (`:1846-1849`) → `collect_modifiers` for actor and target → synthetic bonus modifiers (B11) → per-side `base_block`+`combatant_tags` views → the `AbilityDefense` match (B6–B10) → `ResolutionRequest` (`:1916-1928`) → `resolve` → `damage_for` (B12) → `spend_costs` → health/`Died` → effect attachment (`:2014-2018`) → turn advance. Every `expect` on that path is made unreachable by the start checks and B17.
`CombatError` (`:646-792`): delete `UnsupportedAuthored` and its `Display` arm, add one last variant `TooManyTags` with `Display` `TooManyTags at spec/tags`.
Private helpers, replacing/next to `ability_is_legacy` (`:302-310`) and `rebuild_modifier` (`:1452-1480`):
```rust
fn definition_is_legacy(definition: &CombatDefinition) -> bool;
fn ability_is_legacy(entry: &AbilityDefinition, primary: Ulid) -> bool; // + bonus_stats/damage_dice
fn authored_kernel_stat_surface(spec: &EncounterSpec<'_>) -> bool;
fn authored_grants_active(spec: &EncounterSpec<'_>) -> bool;
fn build_pipeline(definition: &CombatDefinition, policies: &BTreeMap<ModTypeId, StackingPolicy>,
                  interners: &Interners) -> Option<ModifierPipeline>;
fn base_block(definition: &CombatDefinition, combatant: &Combatant,
              interners: &Interners) -> StatBlock;
fn combatant_tags(definition: &CombatDefinition, combatant: &Combatant,
                  interners: &Interners) -> TagSet;
fn collect_modifiers(definition: &CombatDefinition, combatant: &Combatant, round: u32,
                     tag: RollTag, interners: &Interners) -> Vec<Modifier>;
fn modified_int(pipeline: &ModifierPipeline, block: &StatBlock, tags: &TagSet,
                modifiers: &[Modifier], entity: EntityId, stat: StatId) -> i32;
fn bonus_modifiers(names: &[String], pipeline: &ModifierPipeline, block: &StatBlock,
                   tags: &TagSet, modifiers: &[Modifier], entity: EntityId,
                   source: Ulid) -> Vec<Modifier>;
fn damage_for(entry: &AbilityDefinition, outcome: Outcome, rng: &mut DeterministicRng) -> u32;
fn roll_initiative(rule: &InitiativeRule, pipeline: &ModifierPipeline, block: &StatBlock,
                   interners: &Interners, entity: EntityId, authored: i32,
                   rng: &mut DeterministicRng) -> i32;
```
`StatDeclaration` → kernel conversion walks `StatFormula` recursively; the depth is bounded first (B5), and `StatFormula` nodes map one-to-one onto `Expr` (`crates/crpg-rules/src/stats.rs:95-113`). `roll_initiative` folds with an empty `TagSet` and an empty modifier slice: at round zero no attachment exists. Old saves decode because every new container field on `Combatant`, `AbilityDefinition` and `EffectDefinition` carries `#[serde(default)]`; `EffectDefinition`'s field count (`:348`) becomes 7 and `CombatDefinition`'s stays 15/18.

### `crates/crpg-sim/src/world.rs`
Extend the coherence block (`:149-522`) with the checks and messages of B17; no new public item. Insertion points: after the roll-tag and `attribute_names` checks (`:170-198`) the L1–L3 declaration checks; beside the ability checks (`:281-330`) the L6 name checks; beside the effect checks (`:332-366`) the `Stat`-target check; beside the combatant checks (`:400-500`) L4–L5; the L7 tag bound after them, before the timeline/active checks (`:501-522`).

### `crates/crpg-sim/src/lib.rs`
Append `InitiativeRule, StatDeclaration, StatFormula, StatKindDecl, StatLiteral` to the combat re-export (`:33-40`).

Unchanged public signatures: `start_encounter`, `perform_action`, `validate_action`, `legal_actions`, `end_encounter`, `EncounterSpec` (`crates/crpg-sim/src/combat.rs:557-573`), `CombatAction`. `start_encounter_tracked` gains only the T138a parameter.

## Mapping
| T136b data value | Kernel value it becomes | `crpg-sim` code that executes it |
|---|---|---|
| `StatKindWire::Int/Fixed/Bool/Enum/Tags/Dice` (`tasks/T136b.md:95-116`) | `StatKind` (`crates/crpg-rules/src/stats.rs:67-87`) | `StatDeclaration.kind` → `StatDefinition` in `build_pipeline` |
| `StatDecl{name, kind, derived}` (`tasks/T136b.md:118-122`) | `StatDefinition{id, kind, derived}` with `id` the looked-up `StatId` (`crates/crpg-rules/src/stats.rs:115-124`) | `build_pipeline`; ids via `Interners::stat` (`crates/crpg-core/src/intern.rs:152`) |
| `ExprWire` (`tasks/T136b.md:150-172`) | `Expr` (`crates/crpg-rules/src/stats.rs:95-113`) | `StatFormula` → `Expr` conversion |
| `StatValueWire` literal (`tasks/T136b.md:127-145`) | `StatValue` (`crates/crpg-rules/src/stats.rs:50-65`) | `StatLiteral` → `StatValue`; `Tags` names via `Interners::tag` (`crates/crpg-core/src/intern.rs:158`) |
| `Creature.stats` `Fx16_16` (`crates/crpg-data/src/document.rs:117-119`) | `StatBlock` base `Int`/`Fixed` (`crates/crpg-rules/src/stats.rs:232-247`) | `base_block`; `Int` from `Combatant.attributes`, `Fixed` from `fixed_stats`, health from `Combatant.health` (`crates/crpg-sim/src/combat.rs:107-114`) |
| `Ability.bonus_stats` (`tasks/T136b.md:215-219`) | fully modified `Int`, folded into `ModifierTarget::Roll` as `ModOp::Add(StatValue::Int)` (`crates/crpg-rules/src/modifier.rs:54-89`) | `bonus_modifiers`, D7 |
| `DefenseWire::ActorAttribute` (`crates/crpg-data/src/combat.rs:105-107`) | `Against::Dc{value: modified attribute, owner: target}` (`crates/crpg-rules/src/resolution.rs:302-312`) | `modified_int`; today's arm when `stats` is empty |
| `DefenseWire::TargetStat` (`crates/crpg-data/src/combat.rs:108-112`) | `Against::Stat{entity: target, stat}` (`crates/crpg-rules/src/resolution.rs:313-321`) | pipeline query of the target stat |
| `DefenseWire::Constant(v)` (`tasks/T136b.md:190-192`) | `Against::Dc{value: v, owner: target}` | `match` in `perform_action_tracked` |
| `DefenseWire::ActorStat{stat}` (`tasks/T136b.md:194-197`) | `Against::Stat{entity: actor, stat}` | same `match`; actor view |
| `DefenseWire::Opposed{dice, bonus_stats}` (`tasks/T136b.md:198-204`) | `Against::Opposed(RollRequest{actor: target, RollSpec::Dice, natural_die: None, …})` (`crates/crpg-rules/src/resolution.rs:280-299`, `:323`) | same `match`; `bonus_modifiers` on the target |
| `DamageEntry{amount, dice}` (`tasks/T136b.md:207-213`) | `amount` + `DiceExpr` total | `damage_for`; `DiceExpr::evaluate` (`crates/crpg-rules/src/dice.rs:142-170`) |
| `Ruleset.initiative` (`tasks/T136b.md:184-188`) | `DiceExpr` total + Σ modified bonus stats + participant `initiative`, saturating `i32` (`crates/crpg-data/src/combat.rs:385`) | `roll_initiative` at start |
| `EffectTargetWire::Stat{stat}` (`tasks/T136b.md:221-228`) | `ModifierTarget::Stat(StatId)` with `Add`/`Set(Int)` (`crates/crpg-rules/src/modifier.rs:54-62`, `:72-89`) | `collect_modifiers` + `rebuild_modifier` |
| `Effect.grants`, `Creature.tags` (`tasks/T136b.md:230-234`; `crates/crpg-data/src/document.rs:120-121`) | `TagSet` (`crates/crpg-rules/src/stats.rs:22-23`) read by `ConditionExpr::HasTag` (`crates/crpg-rules/src/modifier.rs:96-106`) | `combatant_tags`, passed as `QueryContext.tags` (`crates/crpg-rules/src/modifier.rs:146-156`) |

## Hash and goldens
`state_hash` is unchanged: BLAKE3 over full-`World` serde, no exclusions (`crates/crpg-sim/src/hash.rs:37-49`). New values hash through ordinary serde, all in declaration order:

| Value | JSON key and encoding | Present when |
|---|---|---|
| `CombatDefinition.stats` | `"stats":[{"name":"a","kind":"int"}]` (a derived declaration adds `"derived":{…}`) | kernel surface (B3) |
| `CombatDefinition.initiative` | `"initiative":{"dice":"2d6","bonus_stats":["a"]}` | authored |
| `AbilityDefinition.bonus_stats` | `"bonus_stats":["a"]` | non-empty |
| `AbilityDefinition.damage_dice` | `"damage_dice":[[{"type":"success"},"1d4"]]` | a damage entry authored dice |
| `EffectDefinition.grants` | `"grants":["tag"]` | non-empty |
| `EffectTarget::Stat` | `{"type":"stat","value":{"stat":"a"}}` | authored |
| `AbilityDefense::Constant/ActorStat/Opposed` | adjacent `type`/`value` object | authored |
| `Combatant.fixed_stats` | `"fixed_stats":[["b",65536]]` | a `Fixed` base exists |
| `Combatant.tags` | `"tags":["tag"]` | a referenced effect grants tags |

**Legacy byte-identity (ADR-0029 `:425-431`).** (a) Every new field is skipped at its legacy value, so a legacy-shaped world writes exactly the pre-T137 keys. (b) `definition_is_legacy` equals today's `vectors_are_legacy` (`crates/crpg-sim/src/combat.rs:312-321`) whenever `stats` and `initiative` are empty, so the 15-vs-18 branch is unchanged; srd-lite (already 18 fields via `TargetStat`, `rulesets/srd-lite/heavy.json:25-29`) gains neither field. (c) A legacy-shaped encounter interns nothing new and draws nothing: interning is gated on the kernel stat surface and the grant feature (B3, B15), initiative draws on an authored rule (B13), damage dice on an authored entry (B12). (d) `state_hash`, `history_hash`, `CombatState` (`crates/crpg-sim/src/combat.rs:530-539`) and `Advance` are untouched, and `ActionOutcome.damage` stays the applied damage (`:640-641`) — legacy content reports the same number.

**Interner order on a new surface** (needed for a reproducible new-world hash): on the staged clone, stat names intern in authored stat order, then formula `Tags` names in declaration order and preorder; then the tag interner takes `roll` first, then placement tags in participant order and `Creature.tags` order, then grants in ascending referenced-effect id and authored order. Executing actions interns nothing: `Interners::stat`/`tag` are read-only lookups (`crates/crpg-core/src/intern.rs:152-162`), so a save/load cannot renumber a handle (E014).

**Goldens that must stay byte-identical, unedited:** `crates/crpg-sim/tests/goldens/history_v1.x86_64-pc-windows-msvc.txt` and `.x86_64-unknown-linux-gnu.txt` (`crates/crpg-sim/tests/history_golden.rs:28`); all six `crates/crpg-testkit/goldens/*` (`replay_basic`, `combat_basic`, `srd_lite` × two targets; `crates/crpg-testkit/tests/combat_replay.rs:1015`); every `crpg-cli` replay fixture. A moved golden is a stop, not a re-bless.

**New per-target baseline (H-golden approval asked):** `crates/crpg-sim/tests/goldens/surface_v1.x86_64-pc-windows-msvc.txt` and `.x86_64-unknown-linux-gnu.txt`, one `<step> <64-hex>` line per step, generated independently on each native target with the pinned toolchain under the normal test profile (ADR-0012), never compared with each other. This is the "generalized-surface golden" ADR-0029 `:425-427` names.

**The two shipped configurations.** `rulesets/minimal-d6/` (`rulesets/minimal-d6/ruleset.json:1`, ability `/1`) is wholly legacy: `stats`/`initiative`/`bonus_stats`/`damage_dice`/`grants` all absent, `ActorAttribute`, no effect → 15-field definition, seven-key combatant, `combat_basic` and `replay_basic` unchanged. `rulesets/srd-lite/` (all-`Int` stats, `TargetStat` at `rulesets/srd-lite/heavy.json:25-29`, and a `Roll`-only effect at `rulesets/srd-lite/focus.json:23`, no grants, no bonus stats, no damage dice) is already 18-field today and stays 18-field: `stats` and `initiative` are empty, so the bytes, the transient/persistent stat paths and every number are today's; `srd_lite` and `history_v1` are unchanged. Only content that authors a T136b shape writes a new key.

## Behaviour
"Legacy surface" means `CombatDefinition.stats.is_empty()`; "kernel surface" means it is non-empty. Items are for `crpg-sim` with T138a applied unless marked.

1. **B1 (typed bases).** At start, per participant in authored order and per declared stat in authored order: a non-derived `Int` stat must be present in `Creature.stats` (`MissingStat`) and whole (`raw % 65536 == 0`, else `InvalidStatValue`), and is stored in `Combatant.attributes`; a non-derived `Fixed` stat must be present (`MissingStat`) and its raw value, fractional included, is stored in `fixed_stats` (`tasks/T136b.md:390-393`); a non-derived `Bool`/`Enum`/`Tags`/`Dice` stat is `InvalidStatValue { stat }`; a derived stat needs no base.
2. **B2 (declarations).** `CombatDefinition.stats` holds every declared stat, in authored order, with `kind` and `derived`, exactly when the kernel surface is on; `attribute_names` holds the non-health `Int` names in authored order.
3. **B3 (kernel surface on).** The surface is on iff any declaration is non-`Int` or derived, or any ability has non-empty `bonus_stats`, or any ability uses `ActorStat`, or any `Opposed` has non-empty `bonus_stats`, or any referenced effect targets `Stat`, or `initiative.bonus_stats` is non-empty. Off, behaviour is exactly today's (B6, B7, Hash and goldens).
4. **B4 (derived execution).** On the kernel surface, every stat read goes through `ModifierPipeline::query` (`crates/crpg-rules/src/modifier.rs:543-548`) over the full declaration set, so a derived `Int` `attributes` entry or defence stat evaluates, and `EffectTarget::Stat` modifiers are visible.
5. **B5 (preflight).** At start, on a staged clone of `Interners` (`crates/crpg-core/src/intern.rs:127`), the declared stat names and every tag name inside a formula `Tags` literal are interned (authored declaration order; formula preorder within one declaration). Then the pipeline is built and every declared stat with a base or a formula is queried once per participant in authored order; any kernel error is `InvalidStatValue { stat }` naming that declaration. Each consumer name (ability `attribute`, defence stat, all `bonus_stats`, every `Stat` target, `initiative` bonus stats) is then queried and must yield `StatValue::Int`, else `ValueOverflow`. `ModifierPipeline::new` failure is `InvalidStatValue` naming the first declared stat. A `StatFormula` deeper than 32 or over 4096 nodes is `InvalidStatValue` naming its declaration. The staged clone is published only with the encounter (B18), so a rejection leaves the world's interner unchanged.
6. **B6 (`ActorAttribute`).** Legacy surface: the actor's raw `attributes` value (today, `crates/crpg-sim/src/combat.rs:1862-1871`). Kernel surface: the fully modified actor stat. `Against::Dc{value, owner: target, modifier_target: roll_tag}` (`crates/crpg-sim/src/combat.rs:1885-1890`).
7. **B7 (`TargetStat`).** Legacy surface: the transient single-definition path (today, `crates/crpg-sim/src/combat.rs:1873-1901`). Kernel surface: the fully modified target stat.
8. **B8 (`Constant`).** `Against::Dc{value: v, owner: target, modifier_target: roll_tag}`; no stat is read; the target's DC fold applies.
9. **B9 (`ActorStat`).** `Against::Stat{entity: actor, stat, modifier_target: roll_tag}`; the actor's stat pipeline and then its DC fold apply.
10. **B10 (`Opposed`).** `ResolutionRequest.target = Some(target)`, `Against::Opposed(RollRequest{actor: target, roll: RollSpec::Dice(parsed), bonus_target: roll_tag, natural_die: None, stream: COMBAT_ROLL_STREAM, tags: TagSet::new()})`. Actor dice are drawn first, the opposing dice second, on the same stream (`crates/crpg-rules/src/resolution.rs:612-692`). The target's DC fold does not apply; its roll fold does.
11. **B11 (`bonus_stats`).** For `ActorAttribute`/`Constant`/`ActorStat`/`TargetStat`, the actor's names add to the actor's roll fold; for `Opposed`, the actor's names add to the actor's fold and the defence's names to the target's fold; for `initiative`, the names add to the initiative total. Each name contributes one synthetic modifier: `Add(Int(fully modified value))`, `mod_type = "combat.bonus_stat"`, `StackAll`, `priority 0`, id = lowest unused ULid of that view. When `actor == target` and the defence is `Opposed`, the two name lists are concatenated in authored order into one list with one id sequence, so the single shared view carries no duplicate id and both of its rolls fold the same total. A name that is not a declared non-health `Int` stat is `MissingStat` at start; a referenced effect whose `mod_type` is `combat.bonus_stat` is `InvalidEffect` at start.
12. **B12 (damage dice).** `amount` plus the `DiceExpr` total for the decided outcome, computed in `i64` and clamped once into `0..=u32::MAX`; applied with the existing saturating subtraction and exactly-once `Died` (`crates/crpg-sim/src/combat.rs:1989-2009`). The damage dice are evaluated once, after `resolve` returns and before `spend_costs`, on `combat.roll`; an outcome without a `damage_dice` entry is flat. `ActionOutcome.damage` reports the clamped applied damage.
13. **B13 (initiative).** With a ruleset `initiative` rule, each participant's key is `dice.total.saturating_add(Σ fully modified bonus stat).saturating_add(participant.initiative)` (`crates/crpg-data/src/combat.rs:385`), using the B5 pipeline and the participant's `base_block` (empty tags and modifiers: round zero attaches nothing), one evaluate per participant in authored order on `combat.roll`, in TB and RT starts alike; the result is stored in `Combatant.initiative`. TB start and every TB rollover use it (`crates/crpg-sim/src/combat.rs:1400`, `crates/crpg-sim/src/combat.rs:2181-2189`); RT start keys are `0` (T138 B11); RT→TB reuses it (T138 B25). Without a rule, today's authored key verbatim. `bonus_stats` names are validated like B11.
14. **B14 (`Stat` targets).** `EffectTarget::Stat{stat}` rebuilds as `ModifierTarget::Stat(interners.stat(stat).expect(…))` with `Add`/`Set(Int(value))`; the effect's holder sees it on every fold of that stat while attached. `stat` must be a declared non-health `Int` stat (else `MissingStat` at start). `condition` stays `None` (`crates/crpg-sim/src/combat.rs:1477`); modifier conditions are T139's.
15. **B15 (`grants`).** When any referenced effect grants tags, `Combatant.tags` holds the participant's `Creature.tags` in authored order and `EffectDefinition.grants` holds the authored names, and a combatant's fold tag set is `{placement tags} ∪ {grants of every unexpired attachment of that combatant}`, deduplicated, passed as `QueryContext.tags`. With no grants, both fields are empty and every tag set is empty, exactly today (`crates/crpg-sim/src/combat.rs:1923`, `:1930-1942`). No T137 behaviour reads a tag (no data shape authors `ConditionExpr` yet); the projection and its persistence are the executed contract.
16. **B16 (bridge).** `CombatError::UnsupportedAuthored` does not exist and no start rejects a T136b shape; the C2 trigger list (`tasks/T136b.md:437-445`) all execute. `ability_is_legacy` also requires `bonus_stats.is_empty()` and `damage_dice.is_empty()` (plus T138a's two RT terms).
17. **B17 (load coherence, exact messages).** A combat save is rejected, in this order, when:
    - **L1**, a declaration name is empty (`combat stat declaration carries an empty name`), repeated (`combat stat declarations share a name`), or not interned (`combat stat declaration is not interned`);
    - **L2**, `health_stat` is not a declared non-derived `Int` (`combat health stat is not a declared non-derived int stat`), or `attribute_names` disagrees with the declared non-health `Int` names in authored order (`combat definition declares no attributes`, or the existing `crates/crpg-sim/src/world.rs:174-198` messages);
    - **L3**, a declaration's formula references only its own declarations with matching kinds and passes the depth/node bounds of B5 (`combat stat declarations fail their kernel preflight`), and the rebuilt pipeline accepts the whole declaration set (`combat stats fail their kernel preflight`);
    - **L4**, a combatant's `fixed_stats` names are exactly the declared non-derived non-`Int` non-health names, in authored order (`combatant fixed stats disagree with their definition`), and `attributes` matches `attribute_names` as today (`crates/crpg-sim/src/world.rs:431-443`);
    - **L5**, a combatant `tags` name, a grant name, or a tag name inside a formula `Tags` literal is not interned (`combatant tags are not interned`, `combat effect grants are not interned`, `combat stat tag literal is not interned`);
    - **L6**, a `damage_dice` outcome is absent from `damage` (`combat damage dice name an outcome without flat damage`), a `bonus_stats` or `Stat` target names no declared non-health `Int` stat (`combat ability bonus stats are not declared int stats`, `combat defense stat is not a declared int stat`, `combat effect stat target names no declared int stat`), or `initiative` bonus stats do (`combat initiative bonus stats are not declared int stats`);
    - **L7**, a projected tag set exceeds 1024 (`combat tags exceed their bound`).

    The existing messages at `crates/crpg-sim/src/world.rs:170-198, 281-330, 400-443` are unchanged for the legacy surface, and a legacy save passes them exactly as today.
18. **B18 (start order and transactionality).** Pinned order, extending today's (`crates/crpg-sim/src/combat.rs:910-917`, `:955-1362`, `:1364-1418`): the existing checks; B1/B2 per-stat; B11/B13 name checks; B15 grant checks and the B17 `TooManyTags` bound; the definition. Then, when the kernel surface or grants are on, a staged `Interners` clone is taken and all of the following happen on it: stat names and formula `Tags` names intern; the B5 pipeline and preflight run; the spawn loop runs, with a participant's `initiative` key computed before its timeline insert; placement tags and grants intern; the roll tag interns. Then the head refreshes, and only when nothing has failed are the staged interner and the combat state published. Legacy-shaped starts keep today's direct roll-tag interning (`crates/crpg-sim/src/combat.rs:1402`) and byte order. A rejected start therefore changes nothing: no entity, event, RNG draw, interner entry, or hash. A rejected action is unchanged (today's rule).
19. **B19 (legality).** `validate_action`/`legal_actions` are untouched; costs, order and bounds are as today (`crates/crpg-sim/src/combat.rs:1554-1771`).
20. **B20 (replay).** Two runs of one same-seed script over a new-surface ruleset produce identical per-step `state_hash` values; all new state is hashed (Hash section).
21. **B21 (no invented content).** No production code names a ruleset, attribute, stat, tag, die size or modifier type; `combat.bonus_stat` is the only new constant string and is reserved by D7.

**Not changing** (each is a review stop if touched): `state_hash`/`history_hash` code; `World`'s field order; `Timeline` (no policy, no new method — keys stay `insert`-ed at start, rollover and RT rebuild); `end_turn` and `advance_tracked`'s branches; `SimEvent` (no new variant); `COMBAT_ROLL_STREAM` and `COMBAT_ROLL_TAG`; `CombatAction`; the pinned `CombatError` precedence of every existing path; the `perform_action` draw order for content without damage dice. New state is only ever the fields listed in Hash and goldens.

## Tests
Existing suites (`crates/crpg-sim/tests/combat.rs`, `combat_multi.rs`, `combat_persistence.rs`, `combat_costs.rs`, `combat_self_kill.rs`, `history*.rs`, `world.rs`, `legal_actions.rs`, the testkit and CLI replays) pass unedited apart from T136c's own literals; `crates/crpg-sim/tests/unsupported_authored.rs` (T136c) is deleted with B16, nothing else.

- **S1 (B16, B19, existing goldens)** the suites above plus `crates/crpg-sim/tests/history_golden.rs` and the testkit goldens pass unedited; `git diff --exit-code` over `rulesets`, `campaigns`, `crates/crpg-testkit/goldens`, `crates/crpg-sim/tests/goldens/history_v1.*`.
- **S2 (B1, B2, B5)** `crates/crpg-sim/tests/surface_stats.rs`: one ruleset with `Fixed` (whole and fractional), derived `Int`/`Fixed`, a `Bool`/`Tags` derived literal, a `Bool` non-derived rejection, a missing `Fixed` rejected `MissingStat`, a missing `Int` rejected `MissingStat`, a fractional `Int` rejected `InvalidStatValue`, a divide-by-zero preflight `InvalidStatValue` with the exact stat name.
- **S3 (B2, B3, B6, B7, B16)** `crates/crpg-sim/tests/surface_stats.rs`: a derived `Int` attribute and a derived target-stat defence produce exactly the precomputed values; `attribute_names` excludes the non-`Int` names; the same fixture with `stats` off (all-`Int`) is byte- and number-identical to today; crate-private `ability_is_legacy` cases cover the two new conditions.
- **S4 (B8, B9, B10)** `crates/crpg-sim/tests/surface_resolution.rs`: `Constant` ignores stats and applies the target's DC fold; `ActorStat` reads the actor's modified stat; `Opposed` draws target dice after actor dice with fixed seeds, and its margin equals `actor_total - target_total`.
- **S5 (B11)** `crates/crpg-sim/tests/surface_resolution.rs`: bonus stats add exactly their modified values in both folds; a `HighestBonusWorstPenalty` effect on the same `mod_type` does not suppress them; a reserved `mod_type` effect is `InvalidEffect`; a non-`Int` bonus name is `MissingStat`; synthetic ids never collide with effect ids.
- **S6 (B12)** `crates/crpg-sim/tests/surface_resolution.rs`: `amount + 1d4` (and a negative offset) clamps at 0 and at `u32::MAX`; the report and the health delta agree; `Died` fires once; legacy flat damage is unchanged.
- **S7 (B13)** `crates/crpg-sim/tests/surface_initiative.rs`: rolled keys follow the authored order of draws; ties break on `EntityId`; a TB rollover reuses them; an RT start keys `0`; `start_encounter_in_mode(.., TurnBased)` after RT rebuilds the rolled keys; saturation at `i32::MIN`/`MAX`.
- **S8 (B14, B15)** `crates/crpg-sim/tests/surface_effects.rs`: a `Stat`-target effect changes a later roll by exactly its value while attached and by zero after expiry; `grants` appear in a crate-private tag-set test and survive a save/load; a grant set over 1024 is `TooManyTags` at start; the Display string is pinned.
- **S9 (B17)** `crates/crpg-sim/tests/surface_persistence.rs`: a valid new-surface save round-trips equal; each B17 message is triggered by one raw-JSON mutation of that valid save, with `DeadTarget`-style exact text.
- **S10 (B18)** `crates/crpg-sim/tests/surface_stats.rs`: `state_hash`, the interner lists and the RNG stream state are equal before and after each rejected start (typed-stat, preflight, `bonus_stats`, `Opposed`, damage dice, `grants`, `TooManyTags`).
- **S11 (B20)** `crates/crpg-sim/tests/surface_stats.rs`: one script run twice, equal per-step hashes; a third run after save/load continues identically.
- **S12 (B1–B15, end-to-end)** `crates/crpg-sim/tests/surface_golden.rs` + `crates/crpg-sim/tests/support/surface_schedule.rs` + `crates/crpg-sim/examples/generate_surface_golden.rs`, following `crates/crpg-sim/tests/history_golden.rs:1-70`: one hand-authored ruleset uses every shape — typed and derived stats, all four defence forms, `bonus_stats`, damage dice, rolled initiative, a `Stat`-target effect and `grants` — through `start_encounter`, several `perform_action`s, a save/load and `end_encounter`, all headless; a hand-written value oracle runs before the per-step hashes meet `crates/crpg-sim/tests/goldens/surface_v1.<target>.txt`. Missing baseline fails.
- **S13 (two-keys)** `crates/crpg-sim/tests/surface_contract.rs`, written blind from this file by an OpenCode test writer.
- Docs: `docs/architecture/crpg-sim.md` and `crates/crpg-sim/AGENTS.md` gain the stat/adapter section and the bridge deletion (B16), each signed.

Definition of done per package: `cargo fmt --all`; `cargo clippy -p crpg-sim --all-targets -- -D warnings`; `cargo test -p crpg-sim`; `python tools/lint/deps.py`; `python tools/lint/determinism.py`; `python -m unittest discover -s tools/lint -p "test_*.py"`; the golden `git diff --exit-code` of S1; and, for the batch with T136b/T136c/T138a, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` on Windows/MSVC plus `cargo test -p crpg-sim` and `cargo test -p crpg-testkit` on genuine Linux/GNU.

## Out of scope
Sides, participant tags as authored encounter content, lifetime kinds, tiered thresholds and modifier conditions (deferred by D32, `tasks/T136b.md:13`); stat-added damage; T134/T135 exposure (D1); the time model, RT policy and history v2 (T138); persistent creature state, join and leave (T139); protocol (T140); read-only preview (T143); `crpg.ui/1`; any `crpg-data`, `crpg-rules` or downstream crate edit; any host, script or net change.

## Size budget
- **T137a** (~one session): `combat.rs` ≈ +420/−50 (types, definition serde, stat adapter, pipeline/block/tag helpers, bridge deletion), `world.rs` ≈ +130, `lib.rs` +5, `combat.rs` `#[cfg(test)]` `ability_is_legacy` unit tests ≈ +40; S2–S3, S10, S11 ≈ +550.
- **T137b** (~one session): `combat.rs` ≈ +260/−40 (four defence forms, bonus modifiers, damage dice, initiative, `Stat`/grants on the action path); S4–S9 ≈ +700.
- **T137c** (~half session): schedule ≈ +220, example +30, `surface_golden.rs` ≈ +120, two golden files.
S1 and the docs are inside T137a. No package touches a second crate; if T137b overruns, its exact fallback is T137b1 (B8, B10, B11; S4, S5) then T137b2 (B12–B15; S6–S8), and the golden waits for both.

## Open questions (each with a recommendation)
1. **A non-derived `Bool`/`Enum`/`Tags`/`Dice` stat** has no authored base (D4), yet T136b calls such a document data-clean (`tasks/T136b.md:390-393`, T9 `:494-496`). *Recommend:* reject at start with `InvalidStatValue` (D4), because an executable base would have to be invented. *Alternative:* keep it inert (no key, no error), which hides the trap until T139 reads it. *Alternative:* define defaults from the fixed raw value, which invents semantics.
2. **`grants` has no behaviour consumer in T137** (conditions are deferred, D32). *Recommend:* project and persist as B15, with the projection pinned by a crate-private test and the bytes by a save/load test. *Alternative:* defer the field's execution to T139 and reject granted tags now, which would reject data T136b defines.
3. **Initiative re-rolls.** *Recommend:* roll once at start and reuse it (D9, "stored initiative"). *Alternative:* re-roll every TB round, which needs a draw in `advance_tracked` (`crates/crpg-sim/src/combat.rs:2156-2276`) and changes what a rollover means.
4. **Sim mirrors vs reusing `crpg_data` wire types in hashed state.** *Recommend:* mirrors (D2/D3). *Alternative:* persist `crpg_data::StatDecl`/`ExprWire` verbatim, which is ~140 fewer interface lines but ties sim save bytes to the data schema and breaks the E014 mirror convention.
5. **The generalized-surface golden.** *Recommend:* add it as T137c with the two per-target baselines (Hash and goldens), as ADR-0029 `:425-427` states. *Alternative:* defer it to the T138b/T140 golden round, leaving T137 with only the hand-written oracle.
6. **`TooManyTags` name and location.** *Recommend:* one fieldless variant at `spec/tags` (D12). *Alternative:* reuse `InvalidEffect`, which blames an effect for a union it may not have caused.
7. **Bonus-stat source id.** *Recommend:* the ability id (D7), so provenance is per-ability and future source-scoped removal stays possible. *Alternative:* `Ulid::NIL` for every synthetic modifier, which loses that provenance.
8. **Does the health stat belong in the pipeline?** *Recommend:* yes, as a base (D5). *Alternative:* exclude it and let a data-clean health-referencing formula fail at start.

## Agent log
- 2026-10-23 (UTC) · opencode-go/deepseek-v4.1-flash + T137 draft · Wrote the blind T137 contract from ADR-0029 §1/Follow-on 4, T136b and T138's hand-off, after checking every cited line; it decides T134 is not needed, adds one `TooManyTags` variant while deleting the T136c bridge, and keeps every existing golden byte-identical by gating each new persisted field on the feature that authored it.
