# T056-facts — fact sheet for the T056 MVP AI contract

Fact sheet only, gathered for the Claude architect who writes the T056
contract (MVP AI in `crpg-ai`: move to the nearest hostile and attack,
consuming `legal_actions` only). No recommendations, no proposed types.
Every `path:line` was opened and checked on branch `task/T056-facts-f1`.

## Requirements

From `docs/CRPG_ENGINE_SPEC.md`:

- `Stage 1 — Reactive (MVP). Aggro radius, line of sight, move to nearest hostile, use the first affordable ability. About 200 lines. Ship this.` (`docs/CRPG_ENGINE_SPEC.md:698`)
- `AI may only use the same action-legality API as a player.` with the as-built note that the shared API is `crpg_sim::legal_actions(&World, EntityId) -> Result<Vec<CombatAction>, LegalActionsError>`, sharing `validate_action` with execution, and `AI consumes it, never a net-side copy (D18)` (`docs/CRPG_ENGINE_SPEC.md:723`)
- `AI is server-side, budgeted, and time-sliced. Combat AI computes on turn start with a node budget. Ambient AI ... runs on a round-robin across ticks: at most N entities re-evaluate per tick.` (`docs/CRPG_ENGINE_SPEC.md:724`)
- `Do not build GOAP. Do not build a planner. Do not build learning.` (`docs/CRPG_ENGINE_SPEC.md:719`)
- `No AI beyond "move to nearest hostile and attack".` (MVP out-of-scope list) (`docs/CRPG_ENGINE_SPEC.md:1531`)
- Stage 2 (utility scoring, `For each actor turn, enumerate candidate (ability, target, position) tuples`), stage 3 (behaviour trees for non-combat) and stage 4 (tactical positioning) are named as the later progression, with weights as ruleset/creature data (`docs/CRPG_ENGINE_SPEC.md:700-717`)
- Navigation on the server is a custom deterministic integer crate `crpg-nav` (D27c, ADR-0036, implemented by T035); `Recast and Detour are not used` (`docs/CRPG_ENGINE_SPEC.md:728-736`)
- Dependency placement: `crpg-ai` sits in the `{crpg-net, crpg-ai, crpg-script, crpg-persist}` band reading `crpg-sim` (`docs/CRPG_ENGINE_SPEC.md:195-198`); the spine `core <- data <- rules <- sim <- {net, ai, script} <- server` (`docs/CRPG_ENGINE_SPEC.md:213`); `crpg-ai imports crpg-nav` (`docs/CRPG_ENGINE_SPEC.md:217`)
- Future crate contents: `crpg-ai/ utility scorer, behaviour trees, influence maps` (`docs/CRPG_ENGINE_SPEC.md:1202`)
- Turn order: `Real-time-with-pause and turn-based are both expressed as a Timeline: an ordered queue of (initiative_key, EntityId). In real time the timeline advances every tick; in turn-based it advances on EndTurn.` (`docs/CRPG_ENGINE_SPEC.md:320`); server tick 20 Hz fixed (`docs/CRPG_ENGINE_SPEC.md:318`)
- Every intent is validated against `legal_actions` before execution (`docs/CRPG_ENGINE_SPEC.md:807`)
- The agent task template's constraints include `Deterministic: same world + seed → same decision.` (`docs/CRPG_ENGINE_SPEC.md:1381`)
- The MVP checklist places `one hostile creature` whose creatures use `minimal-d6` (`docs/CRPG_ENGINE_SPEC.md:1517-1518`)
- The creature editor lists `faction, AI profile` among creature identity fields (authoring-side, data) (`docs/CRPG_ENGINE_SPEC.md:1065`)

From `docs/IMPLEMENTATION_PLAN.md`:

- Row T056: `C → I | crpg-ai | T028, T036 | MVP AI: move to nearest hostile and attack, consuming legal_actions only. Opens the crate` (`docs/IMPLEMENTATION_PLAN.md:278`)
- Decision row f: `MVP scope confirmations: IR quest hook instead of Lua; minimal AI in crpg-ai` needed by `T052–T056`, recommendation `Confirm as written in spec §17.3` (`docs/IMPLEMENTATION_PLAN.md:151`); update records `f — confirmed.` (`docs/IMPLEMENTATION_PLAN.md:160`)
- Later AI rows in the same crate: T080 utility scorer with data-driven weights (`docs/IMPLEMENTATION_PLAN.md:352`), T081 AI profiles as data in `crpg-data` (`docs/IMPLEMENTATION_PLAN.md:353`), T082 behaviour trees for schedules (`docs/IMPLEMENTATION_PLAN.md:354`), T083 party AI (`docs/IMPLEMENTATION_PLAN.md:355`), T084 encounter dry-run simulator + CLI wrapper (`docs/IMPLEMENTATION_PLAN.md:356`); T112 automated module playtester builds on T084 (`docs/IMPLEMENTATION_PLAN.md:357`)
- Wave 7 pairs T056 with T026p, T037, T040 (all S) and needs T046/T047 (`docs/IMPLEMENTATION_PLAN.md:412`)

From `tasks/DECISIONS-2026-10-04.md` (D27f):

- `MVP enemy AI is only "move to the nearest hostile and attack", in crpg-ai, consuming legal_actions (T028). There is no scoring, behaviour trees or party AI before M9.` (`tasks/DECISIONS-2026-10-04.md:93-95`)

From `tasks/T028.md`:

- Legality exists `before Phase 9 AI` as `One legality source, shared with perform_action, not a net-only duplicate rules engine` (`tasks/T028.md:5-9`)
- `AI consumption is a later crpg-ai-only task; UI/host filtering gets separate higher-owner tasks.` (`tasks/T028.md:40-42`)
- `Later ai-only consumption needs the exact immutable option/order/bound contract` (`tasks/T028.md:81`)
- The approved surface: `MAX_LEGAL_ACTIONS: usize = 4096`, `validate_action(&World, &CombatAction) -> Result<(), CombatError>`, `legal_actions(&World, EntityId) -> Result<Vec<CombatAction>, LegalActionsError>`, `LegalActionsError::{Actor(CombatError), TooManyOptions { limit }}` (`tasks/T028.md:97-107`)
- `No ActionOption wrapper, serde, diagnostic collection or new CombatError variant. The existing CombatAction is the complete executable option.` (`tasks/T028.md:112-113`)
- Out of scope for T028: `AI scoring/turn scheduling, net legality engine, peer authorization, movement options without an approved sim verb, prediction and script handler registry.` (`tasks/T028.md:69-72`)

## The crate today

- `crates/crpg-ai/Cargo.toml` exists with `name = "crpg-ai"`, `version = "0.1.0"`, workspace edition/license, and **no dependencies at all** (empty `[dependencies]`).
- `crates/crpg-ai/src/lib.rs` is the whole source: `#![forbid(unsafe_code)]`, `#![warn(missing_docs)]`, and the module doc `AI logic: utility-scoring combat decisions, behaviour-tree schedules for non-combat, and (later) influence-map tactical positioning.` — no public items, no code (`crates/crpg-ai/src/lib.rs:1-4`).
- `ALLOWED` table entry: `"crpg-ai": ({"crpg-core", "crpg-rules", "crpg-sim", "crpg-nav"}, set())` — it may import core, rules, sim and nav, with an empty dev-dependency allowance (`tools/lint/deps.py:53`).
- `docs/architecture/crpg-ai.md` does **not** exist; `crates/crpg-ai/AGENTS.md` does **not** exist. (Per AGENTS.md, the first task with real code writes both.)

## legal_actions and actions in sim

- `pub const MAX_LEGAL_ACTIONS: usize = 4096;` (`crates/crpg-sim/src/combat.rs:58`)
- `pub enum CombatAction` — `Copy`, three variants: `UseAbility { actor, ability: Ulid, target: EntityId }`, `EndTurn { actor }`, `Stop { actor }` (`crates/crpg-sim/src/combat.rs:932-952`). There is no `ActionOption` wrapper: "The existing `CombatAction` is the complete executable option" (`tasks/T028.md:113`).
- `pub fn validate_action(world: &World, action: &CombatAction) -> Result<(), CombatError>` — read-only admission, the only pre-mutation path; `perform_action` calls it first (`crates/crpg-sim/src/combat.rs:3453`; contract wording `tasks/T028.md:99-100`, doc wording `crates/crpg-sim/AGENTS.md` invariant 12).
- `pub enum LegalActionsError` — `Actor(CombatError)` (the `EndTurn { actor }` failure; `Stop { actor }` in RT) and `TooManyOptions { limit: usize }` (no partial list) (`crates/crpg-sim/src/combat.rs:3678-3688`).
- `pub fn legal_actions(world: &World, actor: EntityId) -> Result<Vec<CombatAction>, LegalActionsError>` — validates the closing action first, then keeps accepted `UseAbility` candidates over abilities in ascending ULID order × current combatants in ascending `EntityId` order, closing action last; each candidate's own failure excludes only that candidate; a target beyond ability range (or without a mover) fails the last admission stage `OutOfRange` and is omitted (`crates/crpg-sim/src/combat.rs:3733-3736`, doc comment `crates/crpg-sim/src/combat.rs:3711-3732`).
- `pub fn perform_action(world: &mut World, action: &CombatAction) -> Result<Option<ActionOutcome>, CombatError>` — `Ok(Some(outcome))` for an attack, `Ok(None)` for `EndTurn` or an accepted RT `UseAbility` (stores the standing order) (`crates/crpg-sim/src/combat.rs:3803-3808`, doc comment `crates/crpg-sim/src/combat.rs:3800-3802`).
- `pub enum EncounterMode` — `#[default] TurnBased`, `RealTime` (`crates/crpg-sim/src/combat.rs:76-82`); `RT_CLOCK_MAX = i32::MAX` (`crates/crpg-sim/src/combat.rs:86`).
- `pub fn start_encounter(world: &mut World, spec: &EncounterSpec<'_>) -> Result<(), CombatError>` publishes TB (`crates/crpg-sim/src/combat.rs:1443-1445`); `pub fn start_encounter_in_mode(world, spec, mode)` publishes RT (every key `0`, no active turn) and fails `ModeUnsupported` on a ruleset without `round_ticks` (`crates/crpg-sim/src/combat.rs:1454-1457`, doc comment 1447-1453).
- How an action is submitted: there is **no controller trait, `ActorController` or `submit` method anywhere in `crpg-sim`, `crpg-rules` or `crpg-core`** (grep found nothing). The submission path today is the free function `perform_action(&mut World, &CombatAction)` called by the host; T028 says "host disclosure filtering" and AI consumption belong to higher crates (`tasks/T028.md:41-42`).
- Turn-based vs real-time paths: in TB, turn logic lives only in `perform_action` and `despawn` (tick knows nothing of combat); in RT, tick stage 2 runs the clock and stage 3 executes due stored orders through `validate_action` (`crates/crpg-sim/AGENTS.md`, "Encounter mode" section).
- T028's own definition is in `tasks/T028.md`: exact surface at `tasks/T028.md:97-107`, one-validation-path rule at `tasks/T028.md:115-117` (extracted from `perform_action`, both public functions use it), "Options confer no authorization... host always rechecks execution" (`tasks/T028.md:20-30`).

## Movement and targets in sim

- Positions are `[Fx16_16; 3]` (`[x, y, z]`, `y` up, metres); for a creature with a mover, `Transform` is the `f32` mirror written by movement code and read by no rule (`crates/crpg-sim/src/movement.rs` — module doc; mirror `to_f32(v) = raw/65536.0` at `crates/crpg-sim/src/movement.rs:391-393`).
- `pub struct MoveRoute { revision: NavRevision, tag: RouteTag, points: Vec<[Fx16_16; 3]> }` — a whole route the host supplies, replaced whole, never edited (`crates/crpg-sim/src/movement.rs:67-74`); `RouteTag(pub u64)` stored and echoed, never interpreted (`crates/crpg-sim/src/movement.rs:62`).
- `pub struct Mover` (fields private; readers `position()`, `speed()` raw `1..=MAX_SPEED_PER_TICK_RAW`, `step()` stored at creation, `route()`) (`crates/crpg-sim/src/movement.rs:106-133`); `pub struct ActiveRoute` is the route being walked (`crates/crpg-sim/src/movement.rs:177-181`).
- `pub enum Measure { #[default] Euclidean }` — closed, "How rule-visible distance is measured (ADR-0036 S2)" (`crates/crpg-sim/src/movement.rs:79-83`).
- Distance between two actors: `pub fn seg_len(a, b) -> u64` is the *horizontal* `ceil_sqrt(dx^2 + dz^2)` over raw values; `y` is ignored (`crates/crpg-sim/src/movement.rs:310-315`); `pub fn route_measure(measure, points) -> Option<u64>` is the checked route length (`crates/crpg-sim/src/movement.rs:364-368`); `pub fn snap(measure, point)` is the identity in v1 (`crates/crpg-sim/src/movement.rs:384-388`).
- Real-time entry point: `World::set_route(entity, route) -> Result<Option<RouteTag>, MoveError>` fails `AbsentEntity`, `NotMover`, `DeadCombatant`, `TurnBased`, then `validate_route` — "A combatant of a real-time encounter and every non-participant may set a route" (`crates/crpg-sim/src/movement.rs:771-792`). Routes advance per tick in stage 4 (`movement_system`, after clock and timeline), facts via `pub fn tick_with_movement(world) -> Vec<MovementFact>` with `MovementFact::{Arrived{entity, tag}, Cancelled{entity, tag, reason}}` and `CancelReason::Dead` (`crates/crpg-sim/src/tick.rs:32`; `crates/crpg-sim/src/movement.rs:155-171`). There is no `MoveTo` name in sim; the RT verb is `set_route`.
- Turn-based move: `pub fn validate_move(world, actor, route) -> Result<u32, MoveError>` runs `validate_action(EndTurn { actor })` first (one legality source), then `NotMover`, `validate_route`, then the pool cost `ceil(measured/step)*step_cost` against the movement pool (`crates/crpg-sim/src/movement.rs:858-884`); `pub fn perform_move(world, actor, route) -> Result<MoveOutcome, MoveError>` runs `validate_move`, then spends, sets position to the route's last point, clears the route and writes the mirror; keeps the turn, no RNG, no event (`crates/crpg-sim/src/movement.rs:892-897`, doc 886-891). `pub struct TurnMovement { measure, pool: Ulid, step_cost: u32 }` is copied into `CombatDefinition.movement` at start (`crates/crpg-sim/src/movement.rs:90-97`).
- `pub fn route_cost(measured, step, step_cost) -> Option<u32>` (`crates/crpg-sim/src/movement.rs:844`).
- Range check (T149b): `pub fn within_range(measure, range: Fx16_16, from, to) -> bool` — true iff `route_measure(measure, [from, to])` is `Some(d)`, `range.to_raw() >= 0`, and `d <= raw` (`crates/crpg-sim/src/movement.rs:373-381`). `validate_action` ends with the range stage: a missing mover on either side or `!within_range` is `CombatError::OutOfRange`, so out-of-range targets are omitted from `legal_actions` and refused when issued (`crates/crpg-sim/src/combat.rs:3722-3723` doc; `crates/crpg-sim/AGENTS.md`, "Ability range" section). X3: in RT stage 3 a due order failing exactly `OutOfRange` is retained and retried every later tick; any other failure clears the order.
- `Mover` is built at registration and for each *spawned* encounter participant from the ruleset's `movement` rules; without `movement` nothing is built, so a ruleset without `movement` creates no movers and a ranged ability can never be used there (`crates/crpg-sim/AGENTS.md`, "Movement state and operations" + "Ability range" traps).
- `World` accessors: `movers()`, `nav`, `bind_nav(revision)`, `place(entity, position)`, `set_route`, `clear_route` (`crates/crpg-sim/src/movement.rs:713`, `:744`, `:771`).

## Hostility and factions

- **Sim, rules and core have no notion of sides, teams, hostility or allegiance.** Grepping `faction`, `team`, `side`, `hostile`, `Allegiance` over `crates/crpg-sim/src`, `crates/crpg-rules/src` and `crates/crpg-core/src` finds nothing (the `side` hits in `crpg-rules` are dice/roll sides of an opposed check, e.g. `crates/crpg-rules/src/resolution.rs:279-284`, not combat sides).
- **Faction is authored data only, in `crpg-data`:** `Placement.faction: Option<Ulid>` ("Faction reference; explicit null is required when absent") (`crates/crpg-data/src/document.rs:122-125`); the `Faction` document `crpg.faction/1` (`crates/crpg-data/src/document.rs:425-427`) holds `id`, `slug`, `name`, optional `_note`, and `relations: Vec<FactionRelation>` in authored order (`crates/crpg-data/src/document.rs:288-301`); `FactionRelation { faction: Ulid, disposition: i32 }` — "Rules-interpreted integer disposition" (`crates/crpg-data/src/document.rs:306-311`). No sim/rules/runtime code reads `disposition` today (grep finds it only in data round-trip/introspection tests), and `disposition` has no validation rule naming hostility.
- **Sim combatants carry no faction field.** `Combatant` holds `placement: Ulid`, `initiative`, `attributes`, `health`/`max_health`, `dead`, pools, `attached` effects, `order`, `tags`, `fixed`, `entered` (`crates/crpg-sim/src/combat.rs:207-236`). The one sim-side group identity is `Combatant.tags: Vec<String>` ("Creature tags, ascending and distinct... grants are never among them"), carried from the creature prefab, with nothing reading a tag yet (`crates/crpg-sim/src/combat.rs:224-227`; `crates/crpg-sim/AGENTS.md`, "Tags" bullet: "the projection is the contract a condition consumer will read; nothing reads it yet").
- **How combat participants are listed:** authored side, `EncounterParticipant { placement: Ulid, initiative: i32 }` and `Encounter { ruleset: Ulid, participants: Vec<EncounterParticipant> in authored order, respawns }` (`crates/crpg-data/src/combat.rs:752-758` and `763-782`). Runtime side, `Combatant`s are keyed by entity in a `ComponentStore`; "Insertion order is authored participant order, skipped participants omitted, then members, giving the deterministic placement-to-entity mapping" (`crates/crpg-sim/src/combat.rs:202-205`). `legal_actions` pairs abilities with "every current combatant in ascending full `EntityId` order" (`crates/crpg-sim/src/combat.rs:3715-3717`).
- The CLI `demo` groups printed sides as "faction-else-creature groupings" — a presentation choice, not a sim fact (`crates/crpg-cli/AGENTS.md`, invariant 8; map at `crates/crpg-cli/src/demo.rs:123-126`).

## Existing controllers to copy

- **No scripted or automatic controller exists in any production crate.** Grep for `impl .*Controller`, `scripted`, `auto` over `crates/*/src` finds none: sim has no controller type at all (see §3), and no crate implements one.
- The closest existing scripted driver is a test: `crates/crpg-cli/tests/play.rs::direct_fight` (`crates/crpg-cli/tests/play.rs:282-392`) — it loads a fixture campaign through production `crpg-data`, assembles `EncounterSpec` itself, calls `crpg_sim::start_encounter`, then loops while `world.combat().active` is `Some` (guard `< 1000` "the scripted fight must terminate"): the first participant's placement takes `options[0]` from `legal_actions`, every other placement picks the first listed `UseAbility` whose target is a living combatant of a *different placement*, falling back to `options[0]`, and applies it with `crpg_sim::perform_action` (`crates/crpg-cli/tests/play.rs:350-377`).
- The `crpgc play` command's computer rule likewise "picks only from that list" — `crpg_sim::legal_actions` in returned order, performed through `crpg_sim::perform_action` (`crates/crpg-cli/AGENTS.md`, invariant 9).
- `crpg-testkit`'s combat support (`crates/crpg-testkit/tests/support/combat.rs`) loads the fixture campaigns and drives `start_encounter` / `perform_action` through the public sim API (`crates/crpg-testkit/tests/support/combat.rs:19`, `:285`, `:555`); its replay harness (`play_and_verify`) plays recorded intent sequences (`crates/crpg-testkit/src/replay.rs:15`).
- Fixtures an AI test could use: `rulesets/minimal-d6/` (`ruleset.json`, `strike.json`, `roll_under.json`), `rulesets/srd-lite/` (`ruleset.json`, two ability pairs with tables), `campaigns/fixtures/combat_basic/` and `campaigns/fixtures/combat_srd/` (both full campaign roots with `areas/`, `creatures/`, `encounters/`, `rulesets/`, …). The play acceptance tests run over exactly `["combat_basic", "combat_srd"]` (`crates/crpg-cli/tests/play.rs:20-21`), and note about those fixtures: "The fixtures are one combatant per side with distinct prefabs and no factions, so 'the first participant's side' and 'every other living combatant' are exactly the sides the command reports" (`crates/crpg-cli/tests/play.rs:278-281`).

## Determinism rules that bind it

(filled in below)
