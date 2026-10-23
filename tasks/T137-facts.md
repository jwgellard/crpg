# T137 facts — code the adapter generalization will touch

Fact sheet for the T137 contract (`crpg-sim` adapter generalization,
ADR-0029 Follow-on 4). Facts only: no recommendations, no proposed types.
New data shapes come from `tasks/T136b.md` §"Interface" and §"Behaviour"
(T136b is approved but not on this base: `tasks/T136b.md:13`);
everything else cites code on this branch.

## 1. Stat kinds (`StatKind`) and `derived` expressions

T136b adds `StatKindWire::{Fixed, Bool, Enum, Tags, Dice}` plus
`StatDecl.derived: Option<ExprWire>` (`tasks/T136b.md:95-122`);
`health_stat` must name a non-derived `Int`, attributes name `Int`
(`tasks/T136b.md:41-43`); data checks domains, limits, references,
cycles, chain length and kinds (`tasks/T136b.md:320-351`).

- `start_encounter_tracked` (`crates/crpg-sim/src/combat.rs:932`) reads every
  declared stat per creature today: presence, whole-number Fixed check,
  health positivity (`crates/crpg-sim/src/combat.rs:1007-1030`).
- Non-health stats become `Combatant.attributes` (`crpg-sim/src/combat.rs:1033-1038`);
  `Combatant` holds `attributes: Vec<(String, i32)>` (`crates/crpg-sim/src/combat.rs:77`).
- `CombatDefinition.attribute_names` carries non-health names in authored order
  (`crates/crpg-sim/src/combat.rs:1337-1350`).
- Defense preflight builds a transient `StatBlock` + `StatDefinition{id, kind: Int,
  derived: None}` today (`crates/crpg-sim/src/combat.rs:1902-1915`).
- `ability_is_legacy` (`crates/crpg-sim/src/combat.rs:303`) would have to
  account for non-legacy stat shapes.
- Kernel: `StatKind` (`crates/crpg-rules/src/stats.rs:69`),
  `Expr` (`crates/crpg-rules/src/stats.rs:96`),
  `StatDefinition` (`crates/crpg-rules/src/stats.rs:117`),
  `StatValue` (`crates/crpg-rules/src/stats.rs:51`),
  `TagSet` (`crates/crpg-rules/src/stats.rs:23`),
  `SerializableStatValue` (`crates/crpg-rules/src/stats.rs:497`),
  `StatBlock` (`crates/crpg-rules/src/stats.rs:202`).
- Data today: `StatKindWire::Int` only (`crates/crpg-data/src/combat.rs:44`),
  `StatDecl{name, kind}` with no `derived` (`crates/crpg-data/src/combat.rs:52`).
- Tests: `init_missing_stat` (`crates/crpg-sim/tests/combat.rs:173`),
  `init_fractional_stat` (`crates/crpg-sim/tests/combat.rs:205`),
  `init_zero_health` (`crates/crpg-sim/tests/combat.rs:237`),
  `init_too_many_stats` (`crates/crpg-sim/tests/combat.rs:462`),
  undeclared-attribute check (`crates/crpg-sim/tests/combat.rs:369`).
- Rulesets pin `Int`-only stats: `rulesets/minimal-d6/ruleset.json:1`,
  `rulesets/srd-lite/ruleset.json:1`; goldens unchanged (see §Goldens).
- Derived evaluation in sim today: none found.

## 2. Defense / resolution shapes (`Constant`, `ActorStat`, `Opposed`)

T136b extends `DefenseWire` with `Constant(i32)`, `ActorStat{stat}`,
`Opposed{dice, bonus_stats}` (`tasks/T136b.md:190-205`); mapping rows
point at `Against::Dc`, `Against::Stat`, `Against::Opposed`
(`tasks/T136b.md:422-424`).

- `AbilityDefense` today has only `ActorAttribute` / `TargetStat`
  (`crates/crpg-sim/src/combat.rs:140-148`).
- Adapter converts the two current variants
  (`crates/crpg-sim/src/combat.rs:1259-1264`); `TargetStat` presence is
  checked (`crates/crpg-sim/src/combat.rs:1146`).
- `AbilityDefinition.defense` (`crates/crpg-sim/src/combat.rs:235`);
  legacy test is `matches!(entry.defense, ActorAttribute)` plus
  ends_turn/effect/natural_die/costs (`crates/crpg-sim/src/combat.rs:303`).
- Resolution reads actor attribute or target stat
  (`crates/crpg-sim/src/combat.rs:1851-1872`), builds a transient stat view
  for `TargetStat` (`crates/crpg-sim/src/combat.rs:1873-1884`), selects
  `Against::Dc` or `Against::Stat` (`crates/crpg-sim/src/combat.rs:1885-1901`),
  then `RollRequest`/`ResolutionRequest`
  (`crates/crpg-sim/src/combat.rs:1916-1928`) and `resolve`
  (`crates/crpg-sim/src/combat.rs:1951`, `crates/crpg-sim/src/combat.rs:1978`).
- Kernel: `Against` (`crates/crpg-rules/src/resolution.rs:303`),
  `RollRequest` (`crates/crpg-rules/src/resolution.rs:281`),
  `OutcomeTable::evaluate` (`crates/crpg-rules/src/resolution.rs:197`).
- Data today: `DefenseWire::{ActorAttribute, TargetStat}`
  (`crates/crpg-data/src/combat.rs:105`); `Ability.defense`
  (`crates/crpg-data/src/combat.rs:310`).
- Tests: `roll_under_threshold_neighbors`
  (`crates/crpg-sim/tests/combat.rs:514`), `successful_attack_trace`
  (`crates/crpg-sim/tests/combat.rs:540`), failed-attack trace
  (`crates/crpg-sim/tests/combat.rs:570`),
  `target_defense_margin_and_damage`
  (`crates/crpg-sim/tests/combat_multi.rs:345`).
- `Constant` / `ActorStat` / `Opposed` execution in sim today: none found.

## 3. `bonus_stats` (actor roll additions)

T136b adds `Ability.bonus_stats: Option<Vec<String>>`
(`tasks/T136b.md:215-219`); mapping: fully modified `Int` values folded
into `ModifierTarget::Roll` as `ModOp::Add`
(`tasks/T136b.md:421`); semantic check `unknown bonus stat`
(`tasks/T136b.md:375-377`).

- `AbilityDefinition` has no `bonus_stats` field today
  (`crates/crpg-sim/src/combat.rs:227-252`); the adapter copies
  dice/attribute/defense/table/damage/costs/effect/natural_die only
  (`crates/crpg-sim/src/combat.rs:1271-1284`).
- Roll modifiers today come only from attached effects:
  `roll_modifiers` (`crates/crpg-sim/src/combat.rs:1482`),
  `dc_modifiers` (`crates/crpg-sim/src/combat.rs:1511`),
  collected at action time (`crates/crpg-sim/src/combat.rs:1844-1849`).
- Actor roll request carries `bonus_target: roll_tag`, empty `tags`
  (`crates/crpg-sim/src/combat.rs:1916-1924`).
- Kernel: `ModifierTarget::Roll` (`crates/crpg-rules/src/modifier.rs:55`),
  `ModOp::Add` (`crates/crpg-rules/src/modifier.rs:73`),
  `QueryContext` (`crates/crpg-rules/src/modifier.rs:147`).
- Data today: `Ability` has no `bonus_stats`
  (`crates/crpg-data/src/combat.rs:279-324`).
- `bonus_stats` read or folded in sim today: none found.
- Tests pinning roll-modifier behaviour: `effect_adds_three_to_roll`
  (`crates/crpg-sim/tests/combat_multi.rs:389`).

## 4. Damage `dice`

T136b adds `DamageEntry.dice: Option<String>`, `amount` stays required
(`tasks/T136b.md:207-213`); mapping: `amount` + `DiceExpr` total
(`tasks/T136b.md:425`); read-time rule `ability damage dice must be
1..=128 UTF-8 bytes` (`tasks/T136b.md:361-363`).

- `AbilityDefinition.damage: Vec<(Outcome, u32)>` is flat only
  (`crates/crpg-sim/src/combat.rs:239`); adapter maps
  `(outcome, amount)` (`crates/crpg-sim/src/combat.rs:1265-1270`).
- Execution looks up the flat amount for the decided outcome, default 0
  (`crates/crpg-sim/src/combat.rs:1981-1986`), then saturating subtract
  (`crates/crpg-sim/src/combat.rs:1994`), `Died` on positive-to-zero
  (`crates/crpg-sim/src/combat.rs:2004-2009`).
- `ActionOutcome.damage` is the flat amount applied
  (`crates/crpg-sim/src/combat.rs:640`).
- Kernel: `DiceExpr` (`crates/crpg-rules/src/dice.rs:52`),
  `DiceExpr::evaluate` (`crates/crpg-rules/src/dice.rs:142`).
- Data today: `DamageEntry{outcome, amount}` only
  (`crates/crpg-data/src/combat.rs:269`); `Ability.damage`
  (`crates/crpg-data/src/combat.rs:298`).
- Tests: killing-blow terminal (`crates/crpg-sim/tests/combat.rs:677`),
  overkill floors at zero (`crates/crpg-sim/tests/combat.rs:974`),
  `target_defense_margin_and_damage`
  (`crates/crpg-sim/tests/combat_multi.rs:345`).
- Damage-dice evaluation in sim today: none found.

## 5. `initiative` (rolled)

T136b adds `Ruleset.initiative: Option<InitiativeWire{dice, bonus_stats}>`;
the participant's authored key stays and is added as an offset
(`tasks/T136b.md:49-53`, `tasks/T136b.md:174-188`); mapping: `DiceExpr`
total + bonus stats + participant `initiative`, saturating `i32`, as the
timeline key (`tasks/T136b.md:426`).

- `EncounterParticipant.initiative: i32`
  (`crates/crpg-data/src/combat.rs:385`); `Combatant.initiative: i32`
  (`crates/crpg-sim/src/combat.rs:76`), getter
  (`crates/crpg-sim/src/combat.rs:96-98`).
- Adapter copies `participant.initiative` verbatim
  (`crates/crpg-sim/src/combat.rs:1039-1044`).
- Timeline is keyed at authored keys on start
  (`crates/crpg-sim/src/combat.rs:1400`) and on rollover
  (`crates/crpg-sim/src/combat.rs:2185`); ties break on entity id, pinned by
  `initiative_ties_break_on_entity_id`
  (`crates/crpg-sim/tests/combat.rs:608`) and rollover order
  (`crates/crpg-sim/tests/combat.rs:620`).
- `Ruleset` has no `initiative` rule today
  (`crates/crpg-data/src/combat.rs:218-248`).
- Rolled initiative in sim today: none found.

## 6. Effect modifier `Stat` targets

T136b adds `EffectTargetWire::Stat{stat}`, an `Int` non-health stat
(`tasks/T136b.md:58-61`, `tasks/T136b.md:221-228`); mapping:
`ModifierTarget::Stat(StatId)` with `Add`/`Set(Int)`
(`tasks/T136b.md:427`); semantic check `unknown effect stat`
(`tasks/T136b.md:385-389`).

- Sim `EffectTarget` has only `Roll` / `Dc`
  (`crates/crpg-sim/src/combat.rs:160-168`); `EffectModifier.target`
  (`crates/crpg-sim/src/combat.rs:186`).
- Adapter matches only `Roll` / `Dc`
  (`crates/crpg-sim/src/combat.rs:1298-1301`).
- `rebuild_modifier` maps to `ModifierTarget::Roll` / `Dc` with
  `ModOp::Add(Set)(Int)` (`crates/crpg-sim/src/combat.rs:1452-1480`); only
  Roll and Dc folds are collected (`crates/crpg-sim/src/combat.rs:1482-1538`).
- `EffectDefinition` (`crates/crpg-sim/src/combat.rs:199-212`); effects
  copied from `spec.effects` (`crates/crpg-sim/src/combat.rs:1287-1322`);
  attached on success (`crates/crpg-sim/src/combat.rs:2014-2018`) via
  `attach_effect` (`crates/crpg-sim/src/combat.rs:2129-2139`).
- Kernel: `ModifierTarget::Stat` (`crates/crpg-rules/src/modifier.rs:55`),
  `ModOp::{Set, Add}` (`crates/crpg-rules/src/modifier.rs:73`).
- Data today: `EffectTargetWire::{Roll, Dc}`
  (`crates/crpg-data/src/combat.rs:139-144`).
- Tests: `effect_adds_three_to_roll`
  (`crates/crpg-sim/tests/combat_multi.rs:389`),
  `effect_lifetime_across_save`
  (`crates/crpg-sim/tests/combat_multi.rs:533`).
- `Stat`-target modifiers in sim today: none found.

## 7. Effect `grants` (tags)

T136b adds `Effect.grants: Option<Vec<String>>`
(`tasks/T136b.md:230-234`); mapping: `TagSet` of the aimed combatant, read
by `ConditionExpr::HasTag` (`tasks/T136b.md:428`); rule `effect grants
must hold 1..=1024 distinct nonempty tags` (`tasks/T136b.md:366-367`).
Combatant tags already have an authored source, `Creature.tags`
(`tasks/T136b.md:25-27`; `crates/crpg-data/src/document.rs:120-121`).

- `Combatant` carries no tags today (`crates/crpg-sim/src/combat.rs:74-88`);
  `AttachedEffect{effect, expires_round}`
  (`crates/crpg-sim/src/combat.rs:63-68`).
- Resolution passes empty tag sets on both sides today:
  `tags: TagSet::new()` (`crates/crpg-sim/src/combat.rs:1923`),
  `empty_tags` views (`crates/crpg-sim/src/combat.rs:1930-1942`).
- Rebuilt modifiers set `condition: None`
  (`crates/crpg-sim/src/combat.rs:1477`).
- Kernel: `TagSet` (`crates/crpg-rules/src/stats.rs:23`),
  `ConditionExpr::HasTag` (`crates/crpg-rules/src/modifier.rs:97`).
- Data today: `Effect` has no `grants`
  (`crates/crpg-data/src/combat.rs:192-213`).
- Tag grants or tag-conditioned modifiers in sim today: none found.

## T134 dice modes

Today `DiceExpr` is one group plus an integer offset
(`crates/crpg-rules/src/dice.rs:52`); evaluation draws every die in
authored order, applies keep/drop selection, and returns the kept sum
plus offset (`crates/crpg-rules/src/dice.rs:130-142`).
ADR-0029 K3(a) names the kernel gap: dice pools need "count kept dice
with face >= n" (and for botches "count faces <= m"); smallest change is
an evaluation mode on `DiceExpr`, sum by default
(`docs/adr/0029-kernel-range-and-time-model.md:84-89`).
The follow-on table assigns T134 (`crpg-rules`) that mode work, sum
default with notation extension and unchanged trace, implemented with its
first consumer T137 (`docs/adr/0029-kernel-range-and-time-model.md:480`);
`tasks/BACKLOG.md:205` records the same row and dependency.

## T136c bridge

`tasks/T136b.md:430-448` (Companion T136c) states:

- "C1. `CombatError` gains a last variant, with Display
  `UnsupportedAuthored at spec/authored`" (`tasks/T136b.md:431-436`).
- "C2. `start_encounter_tracked` returns `Err(UnsupportedAuthored)`
  immediately after the two data-owned asserts and before the placement
  loop" when a non-`Int`/derived stat, `ruleset.initiative`,
  `bonus_stats`, non-legacy defense, damage dice, `grants`, or a `Stat`
  modifier target is present; "The world stays unchanged"
  (`tasks/T136b.md:437-445`).
- "C3. The matches at `combat.rs:1259-1264` and `:1298-1301` gain arms for
  the new variants: `unreachable!`" (`tasks/T136b.md:446-448`).
- T137 deletes that bridge (`tasks/T136b.md:460-461`).

## T138 hand-off

`tasks/T138.md:846-850` states:

- "**T137 (adapter generalization)** builds on
  `start_encounter_tracked(.., mode)`. It must extend `ability_is_legacy`
  (D13) for every new `AbilityDefinition` field. RT keys stay clock-based.
  An authored initiative spec changes only TB keys and the RT→TB rebuild
  ("stored initiative")."

## Goldens that must not move

- `crates/crpg-sim/tests/goldens/history_v1.x86_64-pc-windows-msvc.txt:1`
  and `crates/crpg-sim/tests/goldens/history_v1.x86_64-unknown-linux-gnu.txt:1`,
  checked by `crates/crpg-sim/tests/history_golden.rs:28`
  (`native_target_golden_replay` with the hand-authored oracle at
  `crates/crpg-sim/tests/history_golden.rs:55`).
- `crates/crpg-testkit/goldens/replay_basic_rust-1.98.0_x86_64-pc-windows-msvc_test-default.golden:1`
  and `crates/crpg-testkit/goldens/replay_basic_rust-1.98.0_x86_64-unknown-linux-gnu_test-default.golden:1`, checked by
  `crates/crpg-testkit/tests/replay_golden.rs:28`
  (`native_target_golden_replay`).
- `crates/crpg-testkit/goldens/combat_basic_rust-1.98.0_x86_64-pc-windows-msvc_test-default.golden:1`
  and `crates/crpg-testkit/goldens/combat_basic_rust-1.98.0_x86_64-unknown-linux-gnu_test-default.golden:1`, checked by
  `crates/crpg-testkit/tests/combat_replay.rs:1015`
  (`native_target_golden_replay`).
- `crates/crpg-testkit/goldens/srd_lite_rust-1.98.0_x86_64-pc-windows-msvc_test-default.golden:1`
  and `crates/crpg-testkit/goldens/srd_lite_rust-1.98.0_x86_64-unknown-linux-gnu_test-default.golden:1`, covered by
  `crates/crpg-testkit/tests/srd_replay.rs:1` (headless srd replay
  integration).
- Pinning suites: `crates/crpg-sim/tests/combat.rs:44`,
  `crates/crpg-sim/tests/combat_multi.rs:326`,
  `crates/crpg-sim/tests/combat_persistence.rs:29`,
  `crates/crpg-sim/tests/combat_costs.rs:71`; fixtures
  `rulesets/minimal-d6/ruleset.json:1`,
  `rulesets/minimal-d6/strike.json:1`,
  `rulesets/srd-lite/ruleset.json:1`.

## Agent log

- 2026-10-23 (UTC) · opencode-go/muse-spark-1.3-contributor + T137 facts · Gathered the sim adapter, kernel API, test, golden and ruleset citations for the seven T136b features plus the T134/T136c/T138/golden closing sections, so two blind contract drafts can build on checked `path:line` facts.
