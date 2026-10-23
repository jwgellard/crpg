# T137 — `crpg-sim` adapter generalization (ADR-0029 Follow-on 4)

## Task / Crate(s) / Status

**`crpg-sim` only; no companion package.** The combat adapter executes the
authored surface T136b adds to `crpg-data` — typed and derived stats, the new
defense shapes, `bonus_stats`, damage `dice`, rolled `initiative`, effect
`Stat` modifier targets and tag `grants` — through the `crpg-rules` kernel,
with every existing replay, history and hash golden byte-identical. It extends
`ability_is_legacy` (T138 hand-off, `tasks/T138.md:846-850`). T134
(success-count and face-count `DiceExpr` modes, `crpg-rules`) is **not** part
of this task (D2); there is no second crate to name. **S-set:** the `World`
struct. **Status: proposed draft** (blind draft A; nothing approved).

Inputs: `docs/adr/0029-kernel-range-and-time-model.md` §"1. Kernel range"
(`:71-178`), §"Compatibility, hash and dependency impact" (`:412-431`),
§"Follow-on contracts" (`:473-494`); `tasks/T136b.md` (approved contract);
`tasks/T137-facts.md` (fact sheet; verified before use); `tasks/T138.md`
(exact-contract form model and hand-off).

## Decision

- **D0. No bridge to delete on this base.** `tasks/T136b.md:430-448`
  specifies a T136c `UnsupportedAuthored` bridge, but this base carries no
  `UnsupportedAuthored` in `crates/crpg-sim/src/combat.rs` (verified by
  search) and `crates/crpg-data/src/combat.rs` already carries every T136b
  shape (`StatKindWire` `:57-75`, `derived` `:80-88`, `InitiativeWire`
  `:178-183`, `DefenseWire` `:231-253`, `EffectTargetWire::Stat` `:279-289`,
  `grants` `:358-360`, `initiative` `:396-398`, `DamageEntry.dice`
  `:425-427`, `bonus_stats` `:478-480`). T137 therefore implements directly.
  *Rejected:* re-introducing a rejection gate first (T017b precedent); there
  is nothing unreachable to guard. If the implementation base shows the
  bridge, the implementer deletes C1–C3 per `tasks/T136b.md:430-448` before
  this contract, with no other change.
- **D1. T134 is deferred, not a companion.** ADR K3(a)
  (`docs/adr/0029-kernel-range-and-time-model.md:84-89`) needs counting modes
  for dice *pools* ("count kept dice with face ≥ n"). T136b's
  `Opposed{dice, bonus_stats}` maps to `Against::Opposed` totals
  (`tasks/T136b.md:424`), and every other T136b dice use is a total
  (initiative, damage). Totals need no mode. *Rejected:* folding T134 into
  T137 (a `crpg-rules` change inside an S-set sim task, against the one-crate
  rule) and blocking T137 on it. T134 lands later as its own one-crate
  package with its first counting consumer.
- **D2. No new `CombatError` variant.** Every new adapter failure reuses an
  existing variant at its existing location: unparseable new dice →
  `InvalidDice` (`combat.rs:743`, `at spec/dice` `:819`); unknown
  bonus/opposed/actor-stat names → `MissingStat` (`:725`, `at spec/stats`
  `:816`); non-derived Dice-kind base on a creature → `InvalidStatValue`
  (`:731`, `at spec/stats` `:817`); unrepresentable sums →
  `ValueOverflow` (`:748`, `at spec/conversion` `:821`). *Rejected:* a
  `what`-carrying variant (new fields in `Display` risk the pinned
  `<Code> at <location>` style, `combat.rs:646-650`).
- **D3. New runtime state persists as symbolic strings plus data wire
  values, never interned handles.** Kernel `StatValue`/`StatBlock` carry no
  serde (`crates/crpg-rules/src/stats.rs:44-50`, `:189-200`), and the
  E014 trap (`crates/crpg-sim/AGENTS.md`) forbids deriving serde over
  `StatId`-keyed maps. `CombatDefinition` therefore persists
  `Vec<crpg_data::StatDecl>` (`crates/crpg-data/src/combat.rs:80-88`;
  all-`String` payloads) and per-combatant non-`Int` bases as
  `Vec<(String, crpg_data::StatValueWire)>` (`:98-116`). The `crpg-data`
  edge is already allowed. *Rejected:* sim-owned mirror enums (a second
  reference-field list, the T013a drift defect) and persisting kernel
  handles (the ADR-0006 renumbering bug).
- **D4. Derived stats evaluate once at encounter start; queries rebuild
  transient views.** The adapter builds kernel `StatDefinition`s
  (`stats.rs:117-124`) from `stat_decls` and one `StatBlock` (`:202`) per
  combatant from base values, evaluates derived stats in authored order,
  and stores results (`Int` into `attributes`, `combat.rs:77`; other kinds
  into the new secondary vector). Action-time queries rebuild transient
  blocks from stored bases plus live `Stat`-target modifiers, following the
  existing `TargetStat` transient-view pattern
  (`tasks/T137-facts.md:61-67`; `combat.rs:1873-1884` per fact sheet).
  *Rejected:* persisting evaluated caches (a second source of truth) and
  re-evaluating from persisted `Expr` on every query without stored bases.
- **D5. Bonus stats fold through the pipeline, not around it.** The T136b
  mapping is normative: fully modified `Int` values as `ModOp::Add` into
  `ModifierTarget::Roll` (`tasks/T136b.md:421`; `modifier.rs:55`, `:73`).
  The adapter declares one synthetic policy entry, mod-type
  `"bonus_stat"` under `StackingPolicy::StackAll`, and folds each listed
  stat in authored order before effect modifiers. *Rejected:* direct integer
  addition outside the pipeline (contradicts the binding mapping row) and
  reusing an effect's mod-type (breaks per-effect policy isolation).
- **D6. Damage dice ride a parallel vector, not a widened tuple.** Both
  `AbilityDefinition.damage` and `CombatDefinition.damage` stay
  `Vec<(Outcome, u32)>` (`combat.rs:239`, `:279`); each gains a sibling
  `damage_dice: Vec<(Outcome, DiceExpr)>` (`dice.rs:52`), joined by outcome
  at execution (`amount` + dice total, saturating `u32`, floored at zero).
  *Rejected:* widening the tuple (rewrites the hand serializer,
  `combat.rs:391-424`, and every legacy byte).
- **D7. Rolled initiative is drawn once at start and stored.**
  `Combatant.initiative` (`combat.rs:76`) keeps holding the timeline key;
  the new `InitiativeSpec` lives on the definition. Rollover
  (`combat.rs:2185`) re-keys the stored value without redrawing, which is
  exactly the T138 "stored initiative" hand-off (`tasks/T138.md:846-850`).
  *Rejected:* redrawing per round (unrepeatable across save/load) and
  per-participant dice (rejected already by `tasks/T136b.md:49-53`).
- **D8. Tags are plumbed state with no outcome effect today.** Modifier
  conditions are deferred (D32; `tasks/T136b.md:13`), and rebuilt modifiers
  keep `condition: None` (`combat.rs:1477`), so no kernel path reads tags
  yet. Combatants still store, hash and pass them, so conditions arrive as
  data-only later. *Rejected:* leaving tags out (T139 could not revive
  them) and inventing a tag-gated rule here (out of scope).

## Interface

Each new field is the **last** field of its struct and each new variant
comes after the existing ones, in the order shown. Every new public item has
a doc comment (`lib.rs:2`, `#![warn(missing_docs)]`). No dependency, no
float, no `HashMap`/`HashSet`, no `unsafe`, no new error variant.

### `crates/crpg-sim/src/combat.rs`

```rust
/// One rolled-initiative rule carried on the definition (T136b mapping
/// tasks/T136b.md:426); absent means authored keys alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitiativeSpec {
    /// Parsed initiative dice.
    pub dice: DiceExpr,
    /// Distinct non-health Int stat names added to the roll, authored order.
    pub bonus_stats: Vec<String>,
}
```

`Combatant` (`:74-88`) gains two last fields:

```rust
    /// Non-Int base stat values in authored stat order; skipped when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    secondary: Vec<(String, crpg_data::StatValueWire)>,
    /// Sorted distinct tag names (creature tags); skipped when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
// in impl Combatant (:90-135), two accessors in field order:
    /// Non-Int base stat values in authored stat order.
    pub fn secondary(&self) -> &[(String, crpg_data::StatValueWire)];
    /// Sorted distinct tag names held by this combatant.
    pub fn tags(&self) -> &[String];
```

`AbilityDefense` (`:140-148`) gains three last variants:

```rust
    /// Uses a constant difficulty value.
    Constant {
        /// The constant difficulty before modifiers.
        value: i32,
    },
    /// Uses the actor's named non-health Int stat value.
    ActorStat {
        /// Declared non-health Int stat name read from the actor.
        stat: String,
    },
    /// Uses the target's opposing roll total.
    Opposed {
        /// Parsed opposing dice.
        dice: DiceExpr,
        /// Distinct non-health Int stat names added to the opposing roll.
        bonus_stats: Vec<String>,
    },
```

`EffectTarget` (`:160-168`) gains a last variant:

```rust
    /// Contributes to the named Int stat of the aimed combatant.
    Stat {
        /// Declared non-health Int stat name.
        stat: String,
    },
```

`EffectDefinition` (`:199-212`) gains a last field:

```rust
    /// Distinct tag names held while attached, authored order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grants: Vec<String>,
```

`AbilityDefinition` (`:227-252`) gains two last fields:

```rust
    /// Distinct non-health Int stat names added to this ability's roll.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bonus_stats: Vec<String>,
    /// Damage dice per outcome, joined with `damage` by outcome.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub damage_dice: Vec<(Outcome, DiceExpr)>,
```

`CombatDefinition` (`:263-300`) gains three last fields:

```rust
    /// Declared stat kinds with derived formulae, authored stat order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stat_decls: Vec<crpg_data::StatDecl>,
    /// Rolled initiative rule; `None` means authored keys alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initiative: Option<InitiativeSpec>,
```

Legacy-shape rule (extends `ability_is_legacy` `:302-310`, T138 D13,
`tasks/T138.md:93-97`):

```rust
fn ability_is_legacy(entry: &AbilityDefinition, primary: Ulid) -> bool {
    matches!(entry.defense, AbilityDefense::ActorAttribute)
        && entry.ends_turn
        && entry.effect.is_none()
        && entry.natural_die.is_none()
        && entry.costs.len() == 1
        && entry.costs[0].0 .0 == primary
        && entry.bonus_stats.is_empty()
        && entry.damage_dice.is_empty()
}

/// Reports whether one effect entry carries only legacy behavior.
fn effect_is_legacy(entry: &EffectDefinition) -> bool {
    entry.grants.is_empty()
        && entry.modifiers.iter().all(|m| matches!(m.target, EffectTarget::Roll | EffectTarget::Dc))
}
```

`vectors_are_legacy` (`:312-321`) additionally requires
`definition.initiative.is_none()`, every `stat_decls` entry to be
`StatKindWire::Int` with `derived: None`, and every effect
`effect_is_legacy`. The hand serializer (`:391-424`) writes exactly today's
keys when legacy; the three new definition fields plus `secondary`,
`tags` and `grants` persist only for non-legacy shapes. The legacy rebuild
(`:454-479`, arms at `:462-470`, `:492`, `:513`) synthesizes all-`Int`
base decls from `attribute_names` (`:277`) plus `health_stat`, `None`
initiative, and empty vectors.

Adapter additions (`start_encounter_tracked`, `combat.rs:932`): stat-decl
conversion after the bounds checks (`:949-954`); initiative draw per
participant in authored order before the timeline insert (`:1400`);
defense arms beside `:1260-1262`; `Stat`-target arm beside the
`Roll`/`Dc` match (`tasks/T137-facts.md:171-172`); `rebuild_modifier`
(`:1453`) maps `Stat{stat}` to `ModifierTarget::Stat` with
`ModOp::Add`/`Set` of `StatValue::Int`. New private helpers, named for
review: `convert_stat_decls`, `evaluate_derived`, `roll_initiative`,
`stat_view` (transient block for one combatant), `tag_union` (base tags
plus attached grants, ascending).

Data-owned asserts (panics, T138-B5 pattern at `combat.rs:955-960`):
`stat derived cycle validated by data read-time checks`,
`stat derived kind validated by data read-time checks`,
`stat derived limits validated by data read-time checks`,
`bonus stat Int validated by data semantic checks`,
`effect stat Int validated by data semantic checks`.

## Mapping

One row per T136b data shape (`tasks/T136b.md:412-428`); the middle column
is the kernel value, the last the sim code that executes it.

| Wire / data value | Kernel value | Executing sim code |
|---|---|---|
| `StatKindWire` (`data:57-75`; `Enum.variants` authored order) | `StatKind` (`rules:stats.rs:69-87`); variants → `BTreeSet<String>` | `convert_stat_decls`; decls on `CombatDefinition.stat_decls` |
| `StatDecl.derived` (`data:80-88`) | `Expr` (`stats.rs:96-113`); `Stat(name)` → interned `StatId` | `evaluate_derived` at start (authored order); transient `stat_view` per query |
| `StatValueWire` (`data:98-116`) | `StatValue` via `SerializableStatValue` (`stats.rs:497-510`) | `Combatant.secondary`; `Int` results stay in `attributes` (`sim:77`) |
| `Ability.bonus_stats[i]` (`data:478-480`) | fully modified `Int`, `ModOp::Add` into `ModifierTarget::Roll` (`modifier.rs:55`, `:73`), mod-type `"bonus_stat"`, `StackAll` | roll fold beside `roll_modifiers` (`sim:1483`); names checked at the `:1146-1153` stat stage |
| `DefenseWire::Constant(v)` (`data:239-240`) | `Against::Dc{value: v, owner: target}` (`resolution.rs:303-312`) | new arm at the `:1886-1890` dispatch; DC fold unchanged |
| `DefenseWire::ActorStat{stat}` (`data:241-245`) | `Against::Stat{entity: actor, stat}` (`resolution.rs:313-321`) | new arm at the `:1852-1891` dispatch; transient actor view |
| `DefenseWire::Opposed{dice, bonus_stats}` (`data:246-252`) | `Against::Opposed(RollRequest{actor: target, …})` (`resolution.rs:281-299`, `:322-324`) | new arm; second draw on `COMBAT_ROLL_STREAM` (`sim:55`) after the actor draw; target bonus folds like the actor's |
| `DamageEntry{amount, dice}` (`data:420-428`) | `amount` + `DiceExpr` total (`dice.rs:52`, `:142`); saturating, floored at zero | `damage_dice` joined by outcome; draw after resolution; `saturating_sub` site (`sim:1994`) |
| `Ruleset.initiative` (`data:396-398`) | dice total + Σ bonus values + authored offset, saturating `i32` | `roll_initiative`; stored in `Combatant.initiative` (`sim:76`); keyed at `:1400`, re-keyed at `:2185` |
| `EffectTargetWire::Stat{stat}` (`data:284-288`) | `ModifierTarget::Stat(StatId)`, `Add`/`Set(Int)` (`modifier.rs:55-89`) | `rebuild_modifier` (`sim:1453`); collected by a new `stat_modifiers` beside `:1483-1538`; read through `stat_view` |
| `Effect.grants`, `Creature.tags` (`data:358-360`; `tasks/T136b.md:25-27`) | `TagSet` (`stats.rs:23`), read by `ConditionExpr::HasTag` (`modifier.rs:97`) | `Combatant.tags` + `EffectDefinition.grants`; `tag_union` passed as `RollRequest.tags` (`resolution.rs:296-298`) replacing `TagSet::new()` (`sim:1923`) and `empty_tags` (`sim:1930-1967`) |

## Hash and goldens

`state_hash` code does not change; nothing is excluded (ADR `:412-431`).
Each new value hashes through ordinary serde with absence-skip:

| Value | Encoding | Present when |
|---|---|---|
| `CombatDefinition.stat_decls` | `"stat_decls":[…]` wire decls | non-legacy shapes |
| `CombatDefinition.initiative` | `"initiative":{"dice":…,"bonus_stats":[…]}` | `Some` |
| `AbilityDefinition.bonus_stats` | `"bonus_stats":[…]` authored order | non-empty |
| `AbilityDefinition.damage_dice` | `"damage_dice":[[outcome, dice],…]` | non-empty |
| `EffectDefinition.grants` | `"grants":[…]` authored order | non-empty |
| `Combatant.secondary` | `"secondary":[[name, value],…]` | non-empty |
| `Combatant.tags` | `"tags":[…]` ascending | non-empty |

A legacy-shape world (all-`Int` base stats, no derived, no initiative,
`ActorAttribute`/`TargetStat` only, flat damage, no `bonus_stats`, no
`Stat` targets, no grants, no creature tags) writes exactly today's bytes:
the legacy predicate holds, the serializer takes the legacy branch, and
no draw touches `COMBAT_ROLL_STREAM`. Hence per-target goldens stay
byte-identical: `history_v1` (both targets), `replay_basic`,
`combat_basic` and `srd_lite` (both targets), with the pinning suites at
`crates/crpg-sim/tests/combat.rs:44`, `combat_multi.rs:326`,
`combat_persistence.rs:29`, `combat_costs.rs:71`
(`tasks/T137-facts.md:254-279`). A missing baseline fails; if any existing
golden moves, that is a stop, not a re-bless (ADR `:425-427`).

## Behaviour

"Authored order" is ruleset/ability/effect document order. New-check
placement: stat-name checks at the existing stat stage (after the
`TargetStat` check, `combat.rs:1146-1153`); new dice parses at the existing
dice stage (`ability.dice.parse`, `:1088`). A rejected start changes
nothing: no entity, event, RNG stream or interner entry (pinned order at
`:937-940`, `validate_action` first at `:1554-1557`).

1. **B1 (decls).** `convert_stat_decls` maps every ruleset stat in authored
   order: `Int/Fixed/Bool/Tags/Dice` to the same kernel kind;
   `Enum{enum_id, variants}` to `StatKind::Enum` with the exact variant set.
   `TooManyStats` (`MAX_COMBAT_STATS`, `:51`) still fires first on count.
2. **B2 (Int path unchanged).** Non-derived `Int` stats, including health,
   execute exactly as today: presence (`MissingStat`, `:1010`), wholeness
   and health positivity (`InvalidStatValue`, `:1015-1027`), checked
   conversion (`ValueOverflow`, `:1020-1027`), `attributes` in authored
   order (`:1033-1038` per fact sheet), names in `attribute_names`
   (`:1350`).
3. **B3 (Fixed).** A non-derived `Fixed` stat is required (`MissingStat`
   when absent) and stored raw (`i32`) in `secondary` under its name. It
   never enters `attributes` or an `Against::Stat` query.
4. **B4 (derived).** Derived stats evaluate once at start in authored order
   through the kernel: `Literal`/`Stat`/six binaries with saturating `Int`
   / `Fixed` arithmetic and flooring division. A `Stat` node reads the
   fully modified value (base plus attached `Stat`-target modifiers).
   `Int` results append to `attributes` in authored order; other kinds to
   `secondary`. Division by zero fails the start with `InvalidStatValue`
   naming the stat.
5. **B5 (Dice base).** A non-derived `Dice`-kind stat on a creature has no
   authored value wire; the adapter fails the start with `InvalidStatValue`
   naming the stat. (Confirm in OQ1.)
6. **B6 (Constant).** `Constant(v)` resolves as `Against::Dc` with
   `owner: target` and the combat roll tag, identical to `ActorAttribute`
   except the base value `v`. Margins, tables, naturals and damage are
   otherwise unchanged.
7. **B7 (ActorStat).** `ActorStat{stat}` resolves as `Against::Stat` on the
   actor through the existing stat pipeline (base + derived + `Stat`
   modifiers + DC fold). Unknown or health stat → `MissingStat`; declared
   non-health non-`Int` → `InvalidStatValue`.
8. **B8 (Opposed).** The target rolls `dice` plus its bonus stats on
   `COMBAT_ROLL_STREAM` immediately after the actor's resolution draw; the
   margin is actor total minus opposed total with no further DC fold. Each
   side applies its own `natural_die` rule to its own trace (`resolve`
   sites `:1951`, `:1978`). Unparseable opposed dice → `InvalidDice`.
9. **B9 (bonus_stats).** Each listed stat must be a declared non-health
   `Int` stat (`MissingStat` otherwise, at `/bonus_stats/<i>` order);
   values are fully modified (derived + `Stat` modifiers) and folded
   ascending as `Add` under mod-type `"bonus_stat"`. Empty list and absent
   are identical (no-op, legacy bytes).
10. **B10 (damage dice).** Applied damage is `amount` plus the dice total,
    saturating `u32`, floored at zero; the draw happens on
    `COMBAT_ROLL_STREAM` after resolution (after the opposed draw when
    opposed). `ActionOutcome.damage` (`:640-641`) is the total applied;
    `Died` fires exactly once on the positive-to-zero transition and
    scheduling removal is unchanged. Unmapped outcomes deal zero with no
    draw. Unparseable damage dice → `InvalidDice` at adapter time.
11. **B11 (initiative).** With `initiative: Some`, each participant's key is
    `saturating_add(dice_total, Σ bonus, authored_offset)` in `i32`,
    drawn once per participant in authored participant order before the
    timeline insert. `None` copies `participant.initiative` (`:1041`)
    verbatim. Ties still break on entity id; rollover re-keys stored values
    (`:2185`) with no redraw. Unparseable initiative dice → `InvalidDice`.
12. **B12 (Stat targets).** A `Stat{stat}` modifier with `Add`/`Set(Int)`
    contributes to the aimed combatant's stat view from its attachment round
    until expiry (`expires_round`, `:63-68`; attachment site per fact sheet
    `:2014-2018`, helper `:2109`). Later rolls, DCs, bonus folds and opposed
    totals read it; `Roll`/`Dc` modifier behavior is unchanged. Unknown or
    health stat → `MissingStat`; non-`Int` declared → `InvalidStatValue`.
    `DuplicateModifier`/`PolicyConflict`/`InvalidEffect` checks
    (`:759-783`) cover `Stat` modifiers unchanged.
13. **B13 (tags).** `Combatant.tags` holds the creature's tags sorted
    ascending and distinct. At query time the request carries
    `base ∪ attached grants` in ascending order. Tags change no margin,
    damage, cost or scheduling today (D8); they are hashed state.
14. **B14 (legacy predicate).** `ability_is_legacy`/`vectors_are_legacy`/
    `effect_is_legacy` are exactly §Interface. A single-ability,
    single-pool, effect-free `Int`-only fight serializes the legacy shape;
    any new field forces the vector shape, and save/load round-trips equal.
15. **B15 (goldens).** Every golden in §Hash and goldens is byte-identical
    per target; `minimal-d6` and `srd-lite` fixtures decode with all new
    fields absent. New-shapes content lives only in new test fixtures.
16. **B16 (precedence).** New failures slot into the pinned orders without
    moving existing ones: start keeps
    `EncounterActive → bounds → data asserts → placement/area/creature →
    stats (B2/B3/B5/B7/B9/B12 names) → dice/table/cost (B8/B10/B11 parses)`
    (`:937-940`); action time keeps `NoEncounter … InsufficientAction`
    (`:1554-1557`, T019 sums first) with defense/bonus/damage/initiative
    already resolved at start, so no action-time variant is added.
17. **B17 (legality surface).** `validate_action`/`legal_actions` signatures
    and enumeration order are unchanged; new shapes flow through the shared
    validator (invariant 12). Affordability still sums same-pool totals in
    template order before any draw.
18. **B18 (history).** No new `SimEvent`, no new `HistoryEvent`, no version
    change: legacy histories hash identically; new-shape fights journal
    today's events only.
19. **B19 (headless proof).** One scripted three-combatant fight on a
    new-shapes ruleset (rolled initiative, one `Opposed` ability, one
    damage-dice ability, one `Stat`-buff effect with a grant) runs headless
    to terminal through `start_encounter`/`perform_action` with a fixed
    seed; two runs give equal per-action `state_hash` sequences, and the
    proof asserts the kill, the buffed follow-up margin shift, and the
    stored tag union contents.

## Tests

Each test names the Behaviour items it proves. Fixtures live in
`crates/crpg-sim/tests/support/mod.rs` beside the existing ones; new-shape
rulesets are new files (existing fixtures are never edited).

- **R1 (B1, B2, B15)** `tests/combat.rs`: all-`Int` plus `Fixed` ruleset
  start; legacy bytes asserted key-by-key; `MissingStat`/`InvalidStatValue`
  neighbors for the new kinds.
- **R2 (B3, B4)** `tests/combat_surface.rs` (new): `Fixed` storage and one
  derived chain (`Add` over two bases) landing in `attributes`/`secondary`
  with exact values; division-by-zero start failure naming the stat.
- **R3 (B5)** `tests/combat_surface.rs`: non-derived `Dice` stat fails
  with `InvalidStatValue`; `state_hash` equal before/after.
- **R4 (B6, B7)** `tests/combat_surface.rs`: `Constant` margin math equals
  the same-valued DC fight; `ActorStat` equals the equivalent target-stat
  fight when values match; both error neighbors (`MissingStat`,
  `InvalidStatValue`).
- **R5 (B8)** `tests/combat_surface.rs`: opposed totals decide the margin;
  RNG draw order equals a scripted two-draw expectation; unparseable
  opposed dice → `InvalidDice` with world unchanged.
- **R6 (B9)** `tests/combat_multi.rs`: bonus fold shifts the margin by the
  exact stat sum; unknown name → `MissingStat`; absent equals empty.
- **R7 (B10)** `tests/combat_surface.rs`: `amount` + dice total applied,
  saturation and zero-floor neighbors, exactly-once `Died`, `damage` field
  equals applied total; bad dice → `InvalidDice`.
- **R8 (B11)** `tests/combat_surface.rs`: stored keys equal
  `saturating(dice + bonus + offset)` for a fixed seed; tie order by entity
  id; rollover keeps stored keys; `None` keeps `:1041` verbatim behavior.
- **R9 (B12)** `tests/combat_multi.rs`: `Add` then `Set` stat buffs change
  the next roll by the exact delta across expiry; dead/health/non-`Int`
  neighbors.
- **R10 (B13)** `tests/combat_surface.rs`: stored tags sorted/distinct;
  grant union contents asserted on the request path; outcomes identical
  with and without tags.
- **R11 (B14, B15)** `tests/combat_persistence.rs`: legacy-shape save/load
  byte equality; each new field forces the vector shape and round-trips;
  legacy predicate unit table for `ability_is_legacy`/`effect_is_legacy`.
- **R12 (B16, B17, B18)** `tests/combat.rs` + `tests/history.rs`: pinned
  precedence neighbors for each new failure (world/RNG/events unchanged);
  `legal_actions` order unchanged on a new-shapes fight; history bytes for
  a legacy fight unchanged.
- **R13 (B19)** `tests/combat_surface.rs`: the end-to-end headless proof —
  a new-shapes ruleset runs to terminal, deterministic per-action hashes,
  kill + buff-shift + tag-union assertions.
- **Two-keys:** `tests/combat_surface_contract.rs`, written blind from this
  contract by the OpenCode test writer.

## Out of scope

Any code (this draft is a proposal); the time model and RT policy (T138);
persistent state, join and leave (T139); protocol (T140); read-only preview
(T143); `crpg.ui/1`; sides, lifetimes, tiered thresholds and modifier
conditions (deferred by D32, `tasks/T136b.md:13`); T134 counting modes (D1);
T135 refill/maximum work; movement (T034/T036); reactions, rest, and
hook dispatch (T053/T055).

## Size budget

About 300–500 lines for this draft. Implementation estimate (for the
adjudicator, not this slot): `combat.rs` ≈ +600/−60, `world.rs` ≈ +40
(extended coherence: decl-shape agreement, initiative-range, tag bound);
tests ≈ +800 over `combat_surface.rs` (new), `combat.rs`, `combat_multi.rs`,
`combat_persistence.rs`, `history.rs`. One implementer session; if it
overruns, split exactly into T137a (stats + defenses + bonus, B1–B9, R1–R6)
and T137b (damage dice + initiative + stat targets + tags, B10–B19,
R7–R13). No doc edits beyond the crate's two files with signed agent-log
entries.

## Open questions

1. **Creature-side defaults for `Bool`/`Enum`/`Tags` (and `Dice`, see B5).**
   *Recommend:* (a) absent `Bool` defaults `false`, absent `Tags` defaults
   empty, absent `Enum` defaults its first variant, non-derived `Dice`
   stats fail per B5 — because T136b B10 requires `missing_stat` only for
   non-derived `Int`/`Fixed`, so the rest must have a total rule, and
   defaults keep prefab burden off existing content. (b) Require all values
   on the creature, which reopens T136b B10.
2. **T134 timing.** *Recommend:* (a) deferred per D1; opposed-as-totals
   already covers the T136b mapping, and no planned ruleset counts faces
   (ADR open question 3, `:503-504`). (b) Land T134 first as its own
   one-crate package, which delays T137 for an unused mode.
3. **`"bonus_stat"` mod-type visibility.** *Recommend:* (a) adapter-private
   entry as specified (D5); hosts never address it, and no wire changes.
   (b) A data-declared policy, which needs a T136b amendment.
4. **Opposed RNG stream.** *Recommend:* (a) second draw on
   `COMBAT_ROLL_STREAM` per B8 (one stream, one tag, ordered draws — the
   K3 "one named stream" shape, `:84-89`). (b) A new stream, which splits
   every legacy-adjacent draw expectation for no new capability.
5. **Damage-dice crit doubling.** *Recommend:* (a) authors express it as a
   per-outcome dice entry (`tasks/T136b.md:207-213`, D8), so the sim adds
   nothing. (b) Kernel doubling, which is a new kernel gap, not a surface
   row.

## Agent log

- 2026-10-23 (UTC) · opencode-go/muse-spark-1.3-contributor + T137 draft · Wrote blind draft A of the T137 adapter-generalization contract from the ADR, T136b, T138 and the fact sheet, verifying every cited path:line on this base (including that the T136c bridge is absent and the T136b shapes are already present in data), so the adjudicator can compare it against the independent draft.
