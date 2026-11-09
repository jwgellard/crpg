# T149 facts — fact sheet for the range-check contract (ADR-0036 X1–X3)

Facts gathered 2026-11-09 on this branch; every citation was opened and
checked on this branch. "Today" means this branch's tree, not a later T036
tree. `crates/crpg-sim/src/movement.rs` does not exist here (the `src/*.rs`
glob lists `area.rs`, `world.rs`, `transform.rs`, `timeline.rs`, `tick.rs`,
`store.rs`, `lib.rs`, `history.rs`, `hash.rs`, `event.rs`, `creature.rs`,
`combat.rs`), so the T036 contract below is the only statement of the
movement surface on this branch.

## Requirements

### ADR-0036 §X

- X1 — "An ability may declare `range: Fx16_16`. Absent means unlimited, so
  existing data is unchanged." (`docs/adr/0036-movement-and-navigation.md:468-469`)
- X2 — "`validate_action` checks, after target liveness, that
  `route_measure(measure, [actor position, target position])` (S2) is at
  most `range`. If either side has no mover, the result is `OutOfRange`.
  `legal_actions` omits out-of-range targets."
  (`docs/adr/0036-movement-and-navigation.md:471-474`)
- X3 — "In RT, a due order whose target is out of range neither executes nor
  ends; it stays due. This amends ADR-0029 T7 for this case only."
  (`docs/adr/0036-movement-and-navigation.md:476-477`)
- D46d — "Range checks X1-X3 come as a follow-on right after T036, as
  recommended." (`docs/adr/0036-movement-and-navigation.md:583-584`)
- Follow-on table row — "new: range (D46d) | `crpg-data`, then `crpg-sim` |
  X1-X3 | T036" (`docs/adr/0036-movement-and-navigation.md:543`)
- S2, the measure seam — "Every rule-visible distance goes through
  `route_measure(measure, points)` with `enum Measure { Euclidean }` ..."
  and "TB cost (C2) and range (X2) call the measure; nothing rule-visible
  calls `seg_len` directly."
  (`docs/adr/0036-movement-and-navigation.md:317-325`, quote at `:317-320`
  and `:324-325`)
- S2 also fixes the v1 value: "`Euclidean` returns the U4 route length."
  (`docs/adr/0036-movement-and-navigation.md:320-321`)

### `docs/CRPG_ENGINE_SPEC.md` — every grep hit for range/reach/distance

- `:549` "semver ranges"; `:701` `distance` in an AI consideration list;
  `:879` client "movement range preview"; `:901` `attack_ranged` slot;
  `:1017` editor `Preview` "compute LOS, ranges".
- Content-reachability lines, not spatial range: `:1042` "reachable
  completion path"; `:1049` "unreachable dialogue"; `:1059` "Unreachable
  nodes"; `:1060` "reachability analysis"; `:1608` "quest reachability";
  `:2013` "unreachable dialogue nodes". `:1141` is "mid-range GPU";
  `:1745` is "sandboxed" with no range meaning.
- No spec sentence states an ability range, a reach value, or a distance
  rule for action admission.

### `docs/adr/0029-kernel-range-and-time-model.md` — every grep hit

- `:14` "the range of the seven rules primitives" (capability range); `:71`
  section title "Kernel range"; `:73-82` K1/K2 capability classing;
  `:110` "skill-dependent fumble ranges" (K5, out of scope); `:420` a
  GDExtension compatibility row. No ADR-0029 sentence states a spatial
  ability range, reach or distance.

## Movement contract it builds on

- From `tasks/T036.md`, `Measure` — `enum Measure { Euclidean }`, closed, `Default`, snake_case
  serde (`tasks/T036.md:236-243`). `route_measure(measure, points) ->
  Option<u64>` is "S2: the rule-visible length of a route"
  (`tasks/T036.md:302-303`) and equals `route_length` for `Euclidean`
  (`tasks/T036.md:449-453`); `route_length` is the sum of segment lengths
  (`tasks/T036.md:300-301`).
- `seg_len(a, b) -> u64` is "U4: horizontal length, `ceil_sqrt(dx^2 +
  dz^2)` over raw values" (`tasks/T036.md:296-297`); `point_at(a, b, len,
  d)` is "U5: the point `d` along a segment of length `len`"
  (`tasks/T036.md:298-299`).
- `Mover` — private `position: [Fx16_16; 3]`, `speed: Fx16_16`,
  `step: Fx16_16`, `route: Option<ActiveRoute>`, read through getters
  (`tasks/T036.md:245-260`); `ActiveRoute` holds `route`, `segment: u16`,
  `along: u64` (`tasks/T036.md:262-275`).
- `World.movers` — `movers: ComponentStore<Mover>` and `nav` on `World`,
  absent when empty/`None` (`tasks/T036.md:326-328`).
- Positions — "Position is the placement's `transform.position`" at mover
  creation (`tasks/T036.md:180-186`); `Transform` is the `f32` mirror
  (`tasks/T036.md:188-191`).
- `TurnMovement` — `measure: Measure`, `pool: Ulid`, `step_cost: u32`,
  persisted as `CombatDefinition.movement: Option<TurnMovement>`
  (`tasks/T036.md:369-382`).
- `route_cost(measured, step, step_cost) -> Option<u32>` is the C4 formula
  (`tasks/T036.md:392-393`); `validate_move(world, actor, route) ->
  Result<u32, MoveError>` is W4's read-only twin
  (`tasks/T036.md:394-395`); `perform_move(world, actor, route) ->
  Result<MoveOutcome, MoveError>` is one atomic TB move
  (`tasks/T036.md:396-397`) that "spends the cost, sets the position to
  the last point, writes the mirror with zero velocity, keeps the turn and
  draws no RNG" (`tasks/T036.md:254-264`).
- `HistoryWorld::perform_move(...)` journals exactly
  `[Moved { entity: actor, points: route.points.clone() }]`
  (`tasks/T036.md:404-406`, `tasks/T036.md:657-660`).
- T036 explicitly leaves T149 out: "**T149**: ability `range` and the X1–X3
  checks; it reads `Mover` and `route_measure` from this task."
  (`tasks/T036.md:1070-1071`)

## Abilities in data today

### `Ability` and its fields

- `Ability` is a struct with `#[serde(deny_unknown_fields)]`
  (`crates/crpg-data/src/combat.rs:583-586`). Identity/display fields:
  `id: Ulid`, `slug`, `name`, optional `_note` (`:589-596`). Rules fields:
  `dice`, `attribute`, `outcome_table`, `damage`, `cost`, `extra_costs`,
  `ends_turn` (`:598-611`). Selection/optional fields: `effect`,
  `defense`, `natural_die`, `requires_target`, `allow_self_target`,
  `recovery_ticks`, `repeats_in_real_time`, `bonus_stats`
  (`:613-633`), all optional-missing except `defense`, `requires_target`
  and `allow_self_target`.
- `DamageEntry` — `outcome`, `amount: u32`, `dice: Option<String>`
  optional-missing (`crates/crpg-data/src/combat.rs:571-581`);
  `AbilityCost` — `pool: Ulid`, `amount: u32` (`:258-266`).
- No `range`, `reach` or `distance` field exists on `Ability`
  (`crates/crpg-data/src/combat.rs:586-634`).

### Serde and schema tags

- `Document::Ability` is tagged `crpg.ability/2`
  (`crates/crpg-data/src/document.rs:434-436`); `Ruleset` is
  `crpg.ruleset/2` (`crates/crpg-data/src/document.rs:431-433`).
- Generated schema entries: `Ruleset` -> `crpg.ruleset/2`,
  `crates/crpg-data/src/schema.rs:116`; `Ability` -> `crpg.ability/2`,
  `crates/crpg-data/src/schema.rs:117`.
- The migration registry has `crpg.ability` at 2 with one `1 -> 2` edge
  (`crates/crpg-data/src/migrations/mod.rs:78`) and `crpg.ruleset` at 2
  with one `1 -> 2` edge (`crates/crpg-data/src/migrations/mod.rs:94`);
  `SchemaVersion` and the registry are at
  `crates/crpg-data/src/migrations/mod.rs:26-36` and
  `crates/crpg-data/src/migrations/mod.rs:101-107`.
- The optional-missing list already includes `effect`, `natural_die`,
  `recovery_ticks`, `repeats_in_real_time`, `bonus_stats`, damage `dice`,
  `movement` and its `measure` (`crates/crpg-data/AGENTS.md:33-40`); the
  current-tag additive rule is `crates/crpg-data/AGENTS.md:41-46`.

### Read-time and semantic validation of an ability

- Read-time dispatch: `validate_local` sends `Document::Ability` to
  `validate_combat_local` (`crates/crpg-data/src/document.rs:501-509`).
- `validate_combat_local`'s `Ability` arm is
  `crates/crpg-data/src/combat.rs:1563-1636`; its checks, in order, are
  slug/name, dice length, attribute, damage uniqueness, extra-cost amounts,
  total spend, defense stat, `recovery_ticks`, `bonus_stats`, `Opposed`
  dice/names, and damage dice (same citation).
- `walk_combat` (`crates/crpg-data/src/validation.rs:581`) checks per
  ruleset (`:609`) and per listed ability (`:625`): attribute membership
  `(:633-649)`, defense stat kind `(:650-679)`, bonus stats `(:680-711)`,
  effect `Stat` names `(:712-737)`, the RT-fields warning `(:738-757)`,
  costs `(:758-781)` and extra pool/cost `(:782-809)`.
- A new optional ability field is decoded by the `Ability` deserializer
  (`crates/crpg-data/src/combat.rs:583-634`), checked in that read-time arm
  (`crates/crpg-data/src/combat.rs:1563-1636`), checked semantically in
  `walk_combat` (`crates/crpg-data/src/validation.rs:625`), and its schema
  is regenerated from the struct (`crates/crpg-data/src/schema.rs:117`,
  `schemas/ability.schema.json:1`).
- `schemas/ability.schema.json` has `allow_self_target` at `:274`,
  `requires_target` at `:375`, and no `"range"` property (grep of the file).

### The T148 movement block

- `Ruleset.movement` is an optional field, last on `Ruleset`
  (`crates/crpg-data/src/combat.rs:410-412`), skipping when `None`; the
  source type is `Option<Box<MovementWire>>` (`:412`) while T148's
  interface text writes `Option<MovementWire>` (`tasks/T148.md:173-177`).
- `MovementWire` is `{ speed: LengthWire, measure: Option<MeasureWire>,
  turn: TurnMovementWire }` (`crates/crpg-data/src/combat.rs:492-503`);
  `TurnMovementWire` is `{ pool: Ulid, step: LengthWire, step_cost: u32 }`
  (`:505-516`); `LengthWire` is `Constant(Fx16_16)` / `Stat(String)`
  (`:518-531`); `MeasureWire` is the closed snake_case `Euclidean` with a
  hand-written decoder (`:533-552`).
- `MAX_SPEED_PER_TICK_RAW: i32 = 1 << 18`
  (`crates/crpg-data/src/combat.rs:53-54`).
- `validate_ruleset_movement` checks speed `1..=MAX_SPEED_PER_TICK_RAW`,
  the pool template, positive step and `step_cost >= 1`
  (`crates/crpg-data/src/combat.rs:1268-1320`), and is called after
  `validate_ruleset_kernel` (`crates/crpg-data/src/combat.rs:1560-1561`).
  The pool is inventoried at `/movement/turn/pool`
  (`crates/crpg-data/src/inventory.rs:792-799`); the creature speed/step
  rule is in `walk_combat`
  (`crates/crpg-data/src/validation.rs:898-926`).
- T148 scope and the "no version bump" reading are
  `tasks/T148.md:14-27` and `tasks/T148.md:70-84`; its wire example is
  `tasks/T148.md:189-200`; its tests are
  `crates/crpg-data/tests/movement_fields.rs:1` and
  `crates/crpg-data/tests/movement_fields_contract.rs:1`.

### Every `Fx16_16` field and its JSON/RON form

- `Fx16_16` serde is `#[serde(transparent)]` over a private `i32`; the wire
  value is the raw signed 32-bit integer, not the decimal display string
  (`crates/crpg-core/src/fixed.rs:22-36`; accessors `:52-59`).
- `crpg-data` maps it to `i32` schema surrogates:
  `StatValueWire::Fixed(#[schemars(with = "i32")] Fx16_16)`
  (`crates/crpg-data/src/combat.rs:100-104`) and
  `LengthWire::Constant(#[schemars(with = "i32")] Fx16_16)`
  (`crates/crpg-data/src/combat.rs:526-528`).
- `types::Transform` fields `position`, `rotation`, `scale` are
  `[Fx16_16; 3]` with `#[schemars(with = "[i32; 3]")]`
  (`crates/crpg-data/src/types.rs:232-245`); `types::Bounds` `min`, `max`
  are the same (`crates/crpg-data/src/types.rs:247-257`).
- Sim's `Combatant.fixed` is `Vec<(String, Fx16_16)>`, skipped when empty,
  so its persisted form is a list of name/raw-`i32` pairs
  (`crates/crpg-sim/src/combat.rs:225-228`).
- `Ability` has no `Fx16_16` field today
  (`crates/crpg-data/src/combat.rs:586-634`).
- No `.ron` file exists in the repository (a `find . -name "*.ron" -not
  -path "./target/*"` returned no files); authored documents here are JSON.

## Action validation in sim today

### Signature, order and target liveness

- `pub fn validate_action(world: &World, action: &CombatAction) ->
  Result<(), CombatError>` (`crates/crpg-sim/src/combat.rs:3332`).
- Doc order for `UseAbility`: "`NoEncounter`; live nonparticipant actor;
  live nonparticipant target; `AbsentActor`; `AbsentTarget`; `DeadActor`;
  `DeadTarget`; `OutOfTurn`; `UnknownAbility`; `SelfTarget`; then per-pool
  affordability in template order (checked sums,
  `ValueOverflow`/`InsufficientAction`, T019)."
  (`crates/crpg-sim/src/combat.rs:3316-3320`)
- `NoEncounter` check (`crates/crpg-sim/src/combat.rs:3338`).
- `Stop` path (`:3341-3367`): live nonparticipant, `AbsentActor`,
  `DeadActor`, TB `WrongMode`, RT terminal gate.
- `EndTurn` path (`:3369-3393`): live nonparticipant, `AbsentActor`,
  `DeadActor`, RT `WrongMode`, `OutOfTurn`.
- `UseAbility` path (`:3395-3500`): live nonparticipant actor/target,
  `AbsentActor`, `AbsentTarget`, `DeadActor`, `DeadTarget`, TB
  `OutOfTurn`/RT terminal gate, `UnknownAbility`, `SelfTarget`,
  per-template affordability, unused/unknown pools, derived stat
  evaluation when `definition.derived` is nonempty.
- Target liveness is `:3404-3424`: live non-combatant target
  `NotParticipant` (`:3404-3406`), non-live target `AbsentTarget`
  (`:3410-3412`), dead combatant `DeadTarget` (`:3422-3424`). X2's "after
  target liveness" is the point after `:3424` in the current order.
- `validate_action` "changes nothing (no interning, no RNG stream, no
  event, no spend, no advance)"
  (`crates/crpg-sim/AGENTS.md:135-143`; repeated at
  `crates/crpg-sim/src/combat.rs:3322-3324`).

### Every `CombatError` variant in declaration order

`CombatError` is declared at `crates/crpg-sim/src/combat.rs:962`; its doc
says "Action-time variants check in declaration order" (`:956-960`).

- `NoEncounter` `:964`; `EncounterActive` `:966`
- `NotParticipant { entity }` `:968-971`; `AbsentActor { actor }` `:973-976`
- `AbsentTarget { target }` `:978-981`; `DeadActor { actor }` `:983-986`
- `DeadTarget { target }` `:988-991`; `OutOfTurn { actor, active }` `:993-998`
- `UnknownAbility { ability }` `:1001-1004`; `SelfTarget` `:1006`
- `InsufficientAction { pool, cost, current }` `:1008-1015`
- `MissingPlacement { placement }` `:1017-1020`; `MixedArea { area_a, area_b }` `:1022-1027`
- `MissingCreature { prefab }` `:1029-1032`; `MissingStat { stat }` `:1035-1038`
- `InvalidStatValue { stat }` `:1041-1044`; `InvalidCost { cost, max }` `:1046-1051`
- `InvalidDice` `:1053`; `InvalidOutcomeTable` `:1055`
- `ValueOverflow` `:1058`; `TooManyParticipants` `:1060`; `TooManyStats` `:1062`
- `MissingPool { pool }` `:1064-1067`; `MissingEffect { effect }` `:1069-1072`
- `InvalidEffect { effect }` `:1075-1078`; `DuplicateModifier { id }` `:1080-1083`
- `InvalidNaturalDie { index }` `:1085-1088`; `PolicyConflict { mod_type }` `:1090-1093`
- `AreaMismatch { expected, found }` `:1096-1101`; `ModeUnsupported` `:1104`
- `WrongMode { mode }` `:1107-1110`; `TooManyTags` `:1113`
- `StatEvaluation { stat }` `:1115-1118`; `PlacementInUse { placement }` `:1120-1123`
- `RulesetMismatch { entity }` `:1126-1129`; `NoEntrants` `:1131`
- `NotCreature { entity }` `:1133-1136`; `AlreadyParticipant { entity }` `:1138-1141`
- `EncounterTerminal` `:1143`

There is no `OutOfRange` variant on this branch, and no `ActionError` type
in `crpg-sim` (grep for `ActionError` returns no matches). The other
validation error type is `LegalActionsError` (`Actor` / `TooManyOptions`),
declared at `crates/crpg-sim/src/combat.rs:3505-3517`.

### Callers

- Re-export: `validate_action` is in the crate root export list
  (`crates/crpg-sim/src/lib.rs:36`).
- `perform_action` delegates to `perform_action_tracked`
  (`crates/crpg-sim/src/combat.rs:3630-3635`), whose first statement is
  `validate_action(world, action)?` (`:3648`).
- `legal_actions` validates the closing action first and maps failure to
  `LegalActionsError::Actor` (`crates/crpg-sim/src/combat.rs:3572`), then
  validates every `UseAbility` candidate (`:3604`).
- `execution_stage` validates each due RT order before resolving it
  (`crates/crpg-sim/src/combat.rs:4410`).

### The real-time order path X3 changes

- `StandingOrder { ability: Ulid, target: EntityId, repeat: bool }`
  (`crates/crpg-sim/src/combat.rs:94-103`); a combatant stores it as
  `order: Option<StandingOrder>`, skipped when `None`
  (`crates/crpg-sim/src/combat.rs:218-220`).
- `CombatState` carries `mode: EncounterMode` (skipped when TB),
  `clock: Option<EncounterClock>` and `pending_mode`
  (`crates/crpg-sim/src/combat.rs:820-835`).
- An accepted RT `UseAbility` only stores the order and returns
  `Ok((None, TransitionFacts::default()))`
  (`crates/crpg-sim/src/combat.rs:3679-3694`).
- `execution_stage` runs stage 3 (`crates/crpg-sim/src/combat.rs:4375`):
  it snapshots the timeline entries with `key <= elapsed`
  (`:4386-4391`), reads the actor's `order` (`:4402-4404`), and calls
  `validate_action` (`:4410`).
- Today a validation failure clears the order and reports `OrderEnded`:
  `.order = None; facts.events.push(TickEvent::OrderEnded { actor });
  continue;` (`crates/crpg-sim/src/combat.rs:4410-4417`). That is the
  branch X3 says must not happen for an out-of-range target: X3 says such
  an order "neither executes nor ends; it stays due"
  (`docs/adr/0036-movement-and-navigation.md:476-477`).
- A surviving executed order is re-keyed at
  `min(elapsed + recovery, RT_CLOCK_MAX)` and a one-shot order clears
  (`crates/crpg-sim/src/combat.rs:4419-4458`).
- ADR-0036 W6 states the pre-X3 independence rule: "Moving does not clear
  an RT standing order, and an order does not stop a route. Only X3 changes
  this." (`docs/adr/0036-movement-and-navigation.md:269-270`)
- Tests pinning the current precedence: `crates/crpg-sim/tests/legal_actions.rs:264`
  pins `DeadTarget`; `crates/crpg-sim/tests/legal_actions.rs:337` is
  `action_error_precedence_is_unchanged`;
  `crates/crpg-sim/tests/rt_orders.rs:4-5` says every rung is checked
  through `validate_action`, `perform_action` and `legal_actions`.

## Rulesets and fixtures

- `rulesets/minimal-d6/ruleset.json` lists one ability id ending `K`
  (`rulesets/minimal-d6/ruleset.json:2-4`); the document is
  `rulesets/minimal-d6/strike.json:1-26`, `schema` `crpg.ability/1`
  (`:24`), no `range`.
- `rulesets/srd-lite/ruleset.json` lists `...17` and `...18`
  (`rulesets/srd-lite/ruleset.json:2-5`); `rulesets/srd-lite/heavy.json:1-41`
  and `rulesets/srd-lite/focus.json:1-37` are `crpg.ability/2`
  (`rulesets/srd-lite/heavy.json:39`, `rulesets/srd-lite/focus.json:35`);
  neither declares `range`.
- `campaigns/fixtures/combat_basic/abilities/strike.json:1-26` is the same
  `strike` (`:24`), copied into
  `campaigns/fixtures/combat_basic/rulesets/minimal-d6.json:1-42`
  (abilities at `:2-4`); `campaigns/fixtures/combat_srd/abilities/heavy.json:1-41`
  and `campaigns/fixtures/combat_srd/abilities/focus.json:1-37` match
  srd-lite (`:39` and `:35`).
- Recursive grep over `rulesets/` and `campaigns/fixtures/` for `range`,
  `reach` and `movement` returns no matches: no shipped ruleset or fixture
  declares a range, a reach or the T148 `movement` block.

## Goldens and hashes that could move

### `crates/crpg-sim/tests/goldens/`

Six files, two targets each: `history_v1`, `history_v2` and `surface_v1` at
`crates/crpg-sim/tests/goldens/<name>.x86_64-pc-windows-msvc.txt` and
`crates/crpg-sim/tests/goldens/<name>.x86_64-unknown-linux-gnu.txt`. No
`movement_v1` or `history_v3` golden exists on this branch; T036 specifies
both as new (`tasks/T036.md:105-108`, `tasks/T036.md:922-942`,
`tasks/T036.md:1019-1036`) and says every existing golden must stay byte for
byte unchanged (`tasks/T036.md:109-115`).

### `crates/crpg-testkit/goldens/`

Six files: `combat_basic`, `replay_basic` and `srd_lite`, each at
`crates/crpg-testkit/goldens/<name>_rust-1.98.0_<target>_test-default.golden`
for the two target suffixes. They are `state_hash` summaries: the harness runs
script step, `tick`, then `state_hash`
(`crates/crpg-testkit/src/harness.rs:184-192`); the replay runner records
`state_hash` per tick (`crates/crpg-testkit/src/replay.rs:365-394`).

### Ability persistence and its hash path

- `AbilityDefinition` is the persisted sim form
  (`crates/crpg-sim/src/combat.rs:412-455`); `recovery_ticks`,
  `repeats_in_real_time`, `bonus_stats` and `damage_dice` are optional
  serde fields (`:439-454`). `CombatDefinition` always writes the legacy 15
  fields and appends the vectors only for non-legacy shapes
  (`crates/crpg-sim/src/combat.rs:617-660`).
- `ability_is_legacy` lists the conditions that keep the legacy shape
  (`crates/crpg-sim/src/combat.rs:510-521`); `definition_is_legacy` adds
  the definition-level conditions (`:535-539`). No range condition exists
  there today.
- `World` is hashed by `state_hash`: finite-float check,
  `serde_json::to_vec(world)`, then `blake3::hash`
  (`crates/crpg-sim/src/hash.rs:37-49`). Abilities reach the hash through
  `CombatDefinition` inside `World`, not a separate ability hash.
- `crpg-data`'s canonical writer is `canonical_json`, which reparses and
  emits recursively sorted keys, two-space indent and exactly one LF
  (`crates/crpg-data/src/canonical.rs:71-80`); `write_document` calls
  `validate_local` then `canonical_json`
  (`crates/crpg-data/src/document.rs:495-499`).
- `crpg-data` has no ability-specific `blake3` digest: `blake3` appears in
  `crates/crpg-data/src/action_signatures.rs:225` and `:409` (action bundle
  identity) and `crates/crpg-data/src/package.rs:243` (assets-lock digest),
  not in the ability or ruleset document path.
- An added optional ability field touches the data struct and schema
  (`crates/crpg-data/src/combat.rs:586-634`,
  `crates/crpg-data/src/schema.rs:117`), the data read/semantic checks
  (`crates/crpg-data/src/combat.rs:1563-1636`,
  `crates/crpg-data/src/validation.rs:625`), the sim `AbilityDefinition`
  and its legacy decision (`crates/crpg-sim/src/combat.rs:412-455`,
  `:510-521`), and any world holding it through `state_hash`
  (`crates/crpg-sim/src/hash.rs:37-49`). T036's stated expectation for
  movement — an absent field keeps existing goldens byte-identical — is
  `tasks/T036.md:109-115`.

## Agent log

- 2026-11-09 (UTC) · opencode-go/deepseek-v4.1-flash + T149 facts · Wrote the cited fact sheet for X1–X3, covering the ADR and spec sentences, the T036 movement surface, the data ability and T148 movement shapes, sim validation and RT orders, shipped rulesets/fixtures, and the goldens/hash path.
