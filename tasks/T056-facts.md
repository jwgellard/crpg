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

(filled in below)

## Movement and targets in sim

(filled in below)

## Hostility and factions

(filled in below)

## Existing controllers to copy

(filled in below)

## Determinism rules that bind it

(filled in below)
