# T056-draft-a — blind draft of the T056 MVP AI contract (`crpg-ai`)

**Status: blind draft, not approved.** Written by the drafting agent from
`tasks/T056-facts.md`, the T028 and T036 contracts, `tasks/T149.md` and
`docs/architecture/crpg-sim.md`. The Claude architect writes the final
contract; the user approves it. Nothing here is approved. Every literal and
citation is the drafter's own.

Citation convention: full repository paths with `:line` (or `:a–b`). Bare
numbers are lines of this file only where a file was cited in the same item.

## Task

T056 — MVP AI: move to the nearest hostile and attack, consuming
`legal_actions` only; opens `crpg-ai`
(`docs/IMPLEMENTATION_PLAN.md:278`). Sole crate: `crpg-ai`
(`tools/lint/deps.py:53` allows `crpg-core`, `crpg-rules`, `crpg-sim`,
`crpg-nav`; this draft uses `crpg-core` and `crpg-sim` only). One task =
one crate; no code in any other crate.

The crate today is empty: `crates/crpg-ai/Cargo.toml` has no dependencies
and `crates/crpg-ai/src/lib.rs:1-4` holds only the module doc and the two
crate attributes. `docs/architecture/crpg-ai.md` and
`crates/crpg-ai/AGENTS.md` do not exist, so the first (and only) part
creates them (`AGENTS.md` "Working rules": a crate with no architecture doc
is not ready for agent work).

The AI is server-side, deterministic, and may use only the same
action-legality API a player uses (`docs/CRPG_ENGINE_SPEC.md:723`):
`crpg_sim::legal_actions(&World, EntityId)` (T028), never a net-side copy.
No GOAP, no planner, no learning (`docs/CRPG_ENGINE_SPEC.md:719`); no AI
beyond "move to the nearest hostile and attack"
(`docs/CRPG_ENGINE_SPEC.md:1531`). Utility scoring (T080), behaviour trees
(T082), party AI (T083) and the host driver are out of scope.

## Parts

### Part 1 — `crpg-ai` (MVP decision)

Owns:
- `crates/crpg-ai/Cargo.toml` (add `crpg-core`, `crpg-sim`; no other
  dependency)
- `crates/crpg-ai/src/lib.rs`
- `crates/crpg-ai/src/decision.rs` (new)
- `crates/crpg-ai/tests/decision.rs` (new) and
  `crates/crpg-ai/tests/decision_contract.rs` (new, two-keys writer)
- `docs/architecture/crpg-ai.md` (new)
- `crates/crpg-ai/AGENTS.md` (new)

goldens: none

## Readings

1. **Predicate hostility, not faction.** Sim, rules and core have no sides,
   teams or hostility at runtime; faction is authored data only
   (`tasks/T056-facts.md` "Hostility and factions"; grep over
   `crates/crpg-sim/src`, `crates/crpg-rules/src`, `crates/crpg-core/src`
   finds nothing). The alternative — thread `Faction`/`disposition`
   (`crates/crpg-data/src/document.rs:288-311`) into `Combatant` — is real
   work in `crpg-sim`/`crpg-data` and would make T056 cross-crate, which
   the one-task-one-crate rule forbids; it is a separate task (open
   question 1). T056 therefore takes the hostility answer from the caller
   as a predicate, with a placement-based convenience for the fixtures.

2. **No movement issued by T056.** `crpg-ai` may import `crpg-nav`
   (`tools/lint/deps.py:53`), but a route needs navmesh bytes the `World`
   does not carry (the host binds a `NavRevision` and supplies whole routes;
   `crates/crpg-sim/src/movement.rs:771-792`). The alternative — AI
   pathfinding inside the decision — would duplicate the host's nav and
   coupling. T056 returns the chosen combat action and exposes
   `nearest_hostile` so the host can route toward it; movement stays the
   host's job (open question 2).

3. **`legal_actions` is the only combat-action source.** T028 forbids a
   second predicate and validating by executing on a clone
   (`tasks/T028.md:115-117`); `validate_action` is the single path and
   `perform_action` calls it first (`crates/crpg-sim/src/combat.rs:3453`).
   The alternative — AI inventing moves or attacks — violates "consuming
   legal_actions only" and the no-invented-action rule
   (`crates/crpg-sim/AGENTS.md`, "Combat controller never invents
   actions"). T056 filters `legal_actions`'s output and never calls
   `perform_action`.

4. **Nearest by mover position, horizontal.** Positions are
   `[Fx16_16; 3]` on `Mover` (`crates/crpg-sim/src/movement.rs:106-133`,
   `:116`); the sim's own distance is `seg_len`, which ignores `y`
   (`crates/crpg-sim/src/movement.rs:310-315`). The alternative — using the
   `f32` `Transform` mirror — reads a presentation value and breaks U2
   (`docs/architecture/crpg-sim.md`, "Movement"). A missing mover is
   treated as infinitely far and tie-broken by `EntityId` (open question 3).

5. **`Option<CombatAction>`, not `Result`.** `decide` returns `None` when
   `legal_actions` fails or the option list is empty; the closing action
   (`EndTurn`/`Stop`) is a legal action and is returned in the no-hostile
   case. The alternative — a public `AiError` — adds a type with no extra
   information for MVP and complicates the host; deferred to T080.

6. **No aggro radius / line of sight in MVP.** The spec names them for
   Stage 1 (`docs/CRPG_ENGINE_SPEC.md:698`) but sim has no LOS primitive
   and no aggro state, and adding them would cross into `crpg-sim`/
   `crpg-nav`. The alternative — stubbing them here — would invent
   behaviour. T056 considers every combatant the predicate calls hostile;
   aggro/LOS are open question 5.

7. **No budget or time-slicing.** The spec asks for node budgets and
   round-robin ambient evaluation (`docs/CRPG_ENGINE_SPEC.md:724`), but
   those are scheduling concerns owned by the host tick. T056 is a pure
   synchronous function over `&World`. The alternative — embedding a budget
   counter — has no measurable work to budget in ~200 lines. Deferred to
   the host driver (out of scope).

## Interface

Every public item has a doc comment (`#![warn(missing_docs)]`). Imports are
`crpg_core::EntityId`, `crpg_sim::{CombatAction, World, Mover, seg_len,
legal_actions, LegalActionsError}`.

`crates/crpg-ai/Cargo.toml`:
```toml
[dependencies]
crpg-core = { path = "../crpg-core" }
crpg-sim  = { path = "../crpg-sim" }
```

`crates/crpg-ai/src/lib.rs`:
```rust
#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! MVP AI: nearest hostile, then attack, over `legal_actions` only.

pub mod decision;
pub use decision::{decide, hostile_by_placement, nearest_hostile};
```

`crates/crpg-ai/src/decision.rs`:
```rust
use crpg_core::EntityId;
use crpg_sim::{CombatAction, LegalActionsError, World};

/// Returns the hostile combatant closest to `actor`, or `None`.
///
/// Hostility is decided by `is_hostile(actor, candidate)`; distance is the
/// horizontal `seg_len` between the two movers' positions, or `u64::MAX`
/// when either side has no mover. Ties break by ascending `EntityId`.
pub fn nearest_hostile<H>(
    world: &World,
    actor: EntityId,
    is_hostile: H,
) -> Option<EntityId>
where
    H: Fn(EntityId, EntityId) -> bool;

/// Chooses one action from `legal_actions(world, actor)`.
///
/// Preference order: a `UseAbility` whose target is a hostile, choosing the
/// nearest hostile (distance, then `EntityId`, then ability ULID); otherwise
/// the closing action (`EndTurn` in turn-based, `Stop` in real time) if it
/// is legal; otherwise `None`. Never mutates `world`, never calls
/// `perform_action`, never reads anything but `legal_actions` and mover
/// positions.
pub fn decide<H>(
    world: &World,
    actor: EntityId,
    is_hostile: H,
) -> Option<CombatAction>
where
    H: Fn(EntityId, EntityId) -> bool;

/// Convenience hostility: different `Combatant::placement()`
/// (`crates/crpg-sim/src/combat.rs:240`). Matches the fixture/CLI
/// heuristic (`crates/crpg-cli/tests/play.rs:278-281`).
pub fn hostile_by_placement(
    world: &World,
    actor: EntityId,
    target: EntityId,
) -> bool;
```

`LegalActionsError` is re-exposed only through `legal_actions`'s result; T056
does not wrap it.

## Behaviour

1. **B-1 (target choice).** `decide` enumerates `legal_actions(world, actor)`
   (`crates/crpg-sim/src/combat.rs:3733`). From its `UseAbility` entries it
   keeps those whose `target` is hostile per `is_hostile(actor, target)` and
   is a current combatant (`world.combatants()`, `:1195`). It picks the
   candidate with the smallest `(distance, target, ability)` in that order,
   where `distance` is `seg_len(actor_mover.position(), target_mover.position())`
   (`crates/crpg-sim/src/movement.rs:310-315`, `:116`) or `u64::MAX` when
   either mover is missing. `target` and `ability` compare by their natural
   `Ord` (`EntityId: Ord`, `Ulid: Ord`).

2. **B-2 (attack).** The chosen candidate is returned verbatim as
   `Some(CombatAction::UseAbility { .. })`. It is, by construction, a member
   of `legal_actions`'s result, so it is admissible by `perform_action` on a
   clone of the same world (T028).

3. **B-3 (move then attack, ordering).** T056 never issues a movement verb
   (`set_route`/`perform_move`/`place`) and never mutates `world`. In a
   turn-based encounter an in-range hostile is attacked in one decision; an
   out-of-range hostile is absent from `legal_actions` (T149b,
   `crates/crpg-sim/src/combat.rs` range stage, last admission stage) and so
   is not chosen — the host routes toward `nearest_hostile(world, actor,
   is_hostile)` between decisions and the attack becomes legal once in
   range. In real time the same holds for a stored `StandingOrder`. The host
   must not call `decide` and `perform_move` in the opposite order for the
   same actor in one tick.

4. **B-4 (turn-based path).** In a turn-based encounter `legal_actions`
   closes with `EndTurn { actor }` (`crates/crpg-sim/src/combat.rs:3740-3743`).
   When no hostile is attackable, `decide` returns `Some(EndTurn { actor })`
   if that closing action is legal, ending the actor's turn. `nearest_hostile`
   is independent of mode.

5. **B-5 (real-time path).** In a real-time encounter `legal_actions` closes
   with `Stop { actor }` and lists `UseAbility` entries that store a
   `StandingOrder` (`crates/crpg-sim/src/combat.rs:3725-3727`). `decide`
   returns `Some(UseAbility { .. })` to set an order toward the nearest
   hostile, or `Some(Stop { actor })` when none is attackable. It never
   returns `EndTurn` in real time (that variant is `WrongMode` there).

6. **B-6 (no hostile).** If no current combatant is hostile to `actor`,
   `nearest_hostile` is `None` and no `UseAbility` candidate survives the
   filter; `decide` returns the closing action when legal, else `None`.

7. **B-7 (no legal action).** If `legal_actions(world, actor)` returns
   `Err(LegalActionsError)` (invalid/non-participant/absent/dead/out-of-turn
   actor, or `TooManyOptions`), `decide` returns `None` and `nearest_hostile`
   still runs over `world.combatants()` (it does not depend on legality).

8. **B-8 (unreachable target).** A target beyond the ability's range, or
   without a mover on either side, fails the last admission stage
   `OutOfRange` and is omitted from `legal_actions`
   (`crates/crpg-sim/src/combat.rs:3722-3723`; T149b). It is therefore
   never chosen; `decide` falls through to the closing action or `None`.

9. **B-9 (purity).** `decide` and `nearest_hostile` take `&World`, perform
   no mutation, draw no RNG, enqueue no event, intern nothing, and do not
   call `validate_action` beyond what `legal_actions` already did. They are
   deterministic: same world + same predicate result → same output.

10. **B-10 (hostility convenience).** `hostile_by_placement(world, actor,
    target)` returns `world.combatants().get(actor).map(|a| a.placement()) !=
    world.combatants().get(target).map(|t| t.placement())`, i.e. different
    placement means hostile. If either is not a combatant it returns `false`.

## Errors

No public error type in MVP (reading 5). `decide` returns `None` exactly
when `legal_actions` fails or its result is empty; `nearest_hostile` returns
`None` when no combatant is hostile. `LegalActionsError` (T028,
`crates/crpg-sim/src/combat.rs:3678-3688`) is not re-exported by
`crpg-ai`; the host that needs the reason calls `legal_actions` itself.

## Tests

Integration tests only, under `crates/crpg-ai/tests/`. Each scenario is
built from `crpg-sim`'s public API and the MOVE fixture where positions
matter (`tasks/T036.md` "Shared definitions"; `crates/crpg-sim/tests/support/mod.rs:79`).
`A`, `B`, `C` are the MOVE creatures' entities; `uid(103)` is the standard
ability; `rev = NavRevision::of_bytes(b"mesh-a")`. `hostile` is
`hostile_by_placement` unless stated. Controls use a TB encounter
(`World::new(7)` + `start_encounter`) unless stated; RT uses
`start_encounter_in_mode(..., EncounterMode::RealTime)`.

- **T-1 `nearest_picks_closest` (B-1).** MOVE TB, `bind_nav(rev)`,
  `place(A, p(0,0))`, `place(B, p(10,0))`, add C at `p(0,10)` as a
  third combatant (distinct placement). `nearest_hostile(world, A,
  hostile)` == `Some(B)` (10 m horizontal) over C (≈14.14 m would be
  `rp` distance; horizontal `seg_len(p(0,0), p(0,10))` = 655360 raw, which
  is less than B's 655360? both 655360 → tie). Use C at `p(0,11)` so
  `seg_len` = 720896 > 655360: nearest is B. *Fails if:* distance uses `y`
  or the `f32` mirror, or the tie-break is reversed.

- **T-2 `nearest_tie_breaks_by_entity_id` (B-1).** MOVE with B at
  `p(10,0)` and C at `p(0,10)` (both horizontal distance 655360). With
  entity ids `B < C` (assert the fixture's order), `nearest_hostile(A,
  hostile)` == `Some(B)`; if the ids were reversed the lower id still wins.
  *Fails if:* store order or insertion order is used instead of `EntityId`.

- **T-3 `missing_mover_is_farthest` (B-1, B-10).** Standard fixture
  (no movers) with A, B hostile: both distances `u64::MAX`, tie-break by
  `EntityId`, so `nearest_hostile(A, hostile)` == `Some(B)` when `B < A`? 
  Use `B` as the lower id and assert `Some(B)`. `hostile_by_placement` with
  a non-combatant target returns `false`. *Fails if:* a missing mover is
  treated as distance 0 or unlimited-in-range.

- **T-4 `decide_attacks_nearest` (B-1, B-2).** MOVE TB, B active (initiative
  9, so B holds the first turn), A and B hostile, `bind_nav(rev)`,
  `place` both at their starts. `decide(world, B, hostile)` ==
  `Some(CombatAction::UseAbility { actor: B, ability: uid(103), target: A })`.
  *Fails if:* the target is C or the closing action is returned.

- **T-5 `decide_tie_breaks_target_then_ability` (B-1).** Two legal
  `UseAbility` entries for the same nearest hostile through two abilities
  (add a second ability `uid(104)` to the ruleset with the same cost):
  the lower ULID wins. Assert `ability == uid(103)`. *Fails if:* the
  ability ULID is ignored.

- **T-6 `decide_no_hostile_closes_turn` (B-6).** MOVE TB, single combatant
  A (remove B from the encounter or make B same placement): `decide(world, A,
  hostile)` == `Some(CombatAction::EndTurn { actor: A })`. RT variant:
  `Some(CombatAction::Stop { actor: A })`. *Fails if:* `None` or an attack
  is returned.

- **T-7 `decide_no_legal_action_returns_none` (B-7).** Standard fixture,
  `decide(world, EntityId::new(9999, 1), hostile)` == `None`
  (non-participant/absent actor). Also a dead actor returns `None`.
  *Fails if:* a closing action is fabricated for an invalid actor.

- **T-8 `decide_unreachable_falls_through` (B-8).** MOVE TB with B active,
  `ranged` (T149b style) range raw `1` on `uid(103)` and A at `p(10,0)`:
  `use_ability(B, A)` is `OutOfRange`, so `legal_actions(B)` omits it;
  `decide(world, B, hostile)` == `Some(CombatAction::EndTurn { actor: B })`.
  *Fails if:* the out-of-range pair is chosen or `decide` panics.

- **T-9 `decide_real_time_sets_order` (B-5).** MOVE_RT, RT start, A holds
  the first turn (keys all 0, active `None`); `legal_actions(A)` lists
  `UseAbility { A, uid(103), B }` and `Stop { A }`. `decide(world, A,
  hostile)` == `Some(CombatAction::UseAbility { actor: A, ability: uid(103),
  target: B })`. *Fails if:* `Stop` is preferred or `EndTurn` appears.

- **T-10 `decide_pure` (B-9).** Clone `world` before and after `decide` and
  `nearest_hostile`; serialized bytes and `state_hash` identical; RNG stream
  list unchanged. *Fails if:* any mutation, intern or draw.

- **T-11 `hostile_by_placement_false_for_same` (B-10).** Two combatants
  sharing a placement return `false`; a combatant and a non-combatant return
  `false`. *Fails if:* same placement is treated as hostile.

## What must not change

- No existing golden changes: the six `crates/crpg-testkit/goldens/*`, the
  `crpg-sim` `history_v1/v2/v3`, `movement_v1`, `surface_v1` files, and
  every `crpg-cli`/server/net hash assertion. T056 adds no golden and no
  hash.
- `crpg-sim` public API, `legal_actions`, `CombatAction`, `validate_action`,
  `perform_action`, movement verbs, and `state_hash` are unchanged.
- `crpg-core` and `crpg-rules` are untouched.
- `crpg-contracts` and `rust-toolchain.toml` are untouched.

## Out of scope

- Utility scoring (T080), behaviour trees (T082), party AI / formation
  (T083), the host driver that calls `decide` (T084/T112).
- Aggro radius and line of sight (open question 5).
- Node budgets and time-slicing (open question 6 / host).
- Faction/disposition-based hostility (open question 1; separate task).
- Pathfinding and route issuance (open question 2; host + `crpg-nav`).
- Any code in crates other than `crpg-ai`.

## Open questions (each with a recommendation)

1. **How does T056 learn who is hostile?** Sim/rules/core have no
   sides/teams/hostility; faction is authored data only and `disposition`
   has no hostility threshold (`tasks/T056-facts.md` "Hostility and
   factions"; `crates/crpg-data/src/document.rs:288-311`). *Recommend:*
   keep T056 in one crate by taking `is_hostile: Fn(EntityId, EntityId) ->
   bool` from the caller, plus `hostile_by_placement` as the fixture/CLI
   heuristic. A faction-aware resolver (threading `Faction`/`disposition`
   into combatants and a threshold) is a separate `crpg-sim`/`crpg-data`
   task that T056 can adopt later without an API break, because the
   predicate is the seam.

2. **Does T056 issue movement routes?** `World` carries no navmesh bytes and
   routes are host-supplied (`crates/crpg-sim/src/movement.rs:771-792`).
   *Recommend:* no — T056 returns the combat action and exposes
   `nearest_hostile`; the host routes toward it (B-3). Route computation
   inside AI waits for T080/T084 and a host-provided mesh.

3. **Missing mover distance.** Combatants without movers (rulesets without
   `movement`) have no position. *Recommend:* treat as `u64::MAX` and
   tie-break by `EntityId` (T-3). Alternative: make `nearest_hostile`
   return `None` for an actor with no mover. The `+∞` choice keeps the
   function total and matches "nearest" only among positioned actors.

4. **`Option` vs `Result`.** *Recommend:* `Option<CombatAction>` (reading 5).
   If the host needs the `LegalActionsError`, it calls `legal_actions`
   itself; T056 stays ~200 lines.

5. **Aggro radius and line of sight.** The spec names them for Stage 1
   (`docs/CRPG_ENGINE_SPEC.md:698`) but sim has no LOS primitive and no
   aggro state. *Recommend:* omit both in T056; every combatant the
   predicate calls hostile is considered. A later task adds a
   `crpg-sim`/`crpg-nav` LOS check and an aggro field on the AI profile
   (T081).

6. **Budget and time-slicing.** *Recommend:* T056 is synchronous and
   budget-free; the host schedules calls (reading 7). Budget accounting
   belongs to the host driver (out of scope).

7. **Should `hostile_by_placement` be the default inside `decide`?**
   *Recommend:* no — `decide` requires the predicate explicitly so callers
   cannot accidentally treat same-placement combatants as hostile. A default
   parameter would hide that. `hostile_by_placement` is offered as a
   convenience only.

## Agent log

- 2026-11-15 (UTC) · opencode/beam-free + T056 draft a · Blind draft of the T056 MVP AI contract: predicate hostility, no movement issued, nearest-by-mover-position with EntityId tie-break, Option return, no aggro/LOS/budget; opens crpg-ai with its architecture doc and AGENTS.md. Not approved.