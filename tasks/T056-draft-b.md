# T056 — MVP AI: move to the nearest hostile and attack (`crpg-ai`)

**Status: draft b (blind). Not approved.** Written blind from
`tasks/T056-facts.md` by one model; another model is drafting the same
contract blind. A Claude architect writes the final contract; the user
approves it. Nothing here is approved.

Citation convention: every `path:line` below was opened on branch
`task/T056-draft-b-d1` (base `62b9860`). The fact sheet is
`tasks/T056-facts.md`; a `path:line` quoted from it is cited as
`tasks/T056-facts.md:NN`.

## Task

Spec §6.1 Stage 1 names the reactive MVP: "Aggro radius, line of sight,
move to nearest hostile, use the first affordable ability. About 200 lines.
Ship this." (`docs/CRPG_ENGINE_SPEC.md:698`). The MVP scope cut trims it to
exactly "No AI beyond 'move to nearest hostile and attack'"
(`docs/CRPG_ENGINE_SPEC.md:1531`), and D27f repeats that the MVP enemy AI is
only that, in `crpg-ai`, consuming `legal_actions` (`tasks/DECISIONS-2026-10-04.md:93-95`,
quoted at `tasks/T056-facts.md:36`). T056 opens the crate
(`docs/IMPLEMENTATION_PLAN.md:278`); its prerequisites are T028 (the shared
legality surface) and T036 (movement).

The deliverable is one deterministic, read-only decision function in
`crpg-ai`: given a `World`, an actor it controls, a navmesh, and the set of
hostiles, it picks the nearest hostile, and returns either the legal action
that attacks it, the route that moves toward it, or a pass. Attacks are
chosen only from `crpg_sim::legal_actions` (`docs/CRPG_ENGINE_SPEC.md:723`);
the AI never mutates the world, never invents an action, and never reaches
`crpg-data`. The host applies the returned decision through the existing
public verbs (`perform_action`, `perform_move`/`set_route`), which revalidate
(`tasks/T028.md:20-30`).

## Parts

One part, one crate. Hostility is an *input* (Reading 1), so nothing here
needs a second crate; the only place that could resolve faction data into a
hostile set is the host, which is out of scope (see "Out of scope" and
"Open questions").

### T056 — `crpg-ai` (opens the crate)

Owns (full repository paths):

- `crates/crpg-ai/Cargo.toml` — add `[dependencies]` on `crpg-core`,
  `crpg-sim`, `crpg-nav` (path deps), plus a `[dev-dependencies]` on
  `serde_json` for the JSON world fixtures. `crpg-rules` is permitted by the
  ALLOWED table (`tools/lint/deps.py:53`) but is not added: the MVP decision
  uses no rule type.
- `crates/crpg-ai/src/lib.rs` — the whole implementation (replace the
  four-line skeleton at `crates/crpg-ai/src/lib.rs:1-4`).
- `crates/crpg-ai/tests/decide.rs` — the implementer suite.
- `crates/crpg-ai/tests/decide_contract.rs` — the two-keys writer suite,
  written blind from this file.
- `crates/crpg-ai/tests/fixtures/ai_tb.json` and
  `crates/crpg-ai/tests/fixtures/ai_rt.json` — checked-in `World` JSON
  fixtures (see Tests: fixture).
- `docs/architecture/crpg-ai.md` (new) — the first task with real code
  writes it (`docs/architecture/README.md`; root `AGENTS.md`).
- `crates/crpg-ai/AGENTS.md` (new) — same rule.

`goldens:` none. T056 adds no persisted state, no hash, no golden. The
decision is a pure read of the world, so it changes no existing bytes; the
existing goldens must not move (see "What must not change").

## Readings

Each needs the user's approval with the contract.

1. **Hostility is an explicit input, never derived by the AI.** `crpg-ai`
   may import only `crpg-core`, `crpg-rules`, `crpg-sim`, `crpg-nav`
   (`tools/lint/deps.py:53`), so it cannot read `crpg-data`'s `Faction`
   (`crates/crpg-data/src/document.rs:288`) or `Placement.faction`
   (`crates/crpg-data/src/document.rs:125`). Sim, rules and core hold no
   runtime sides, teams, hostility or allegiance (the fact sheet grep,
   `tasks/T056-facts.md:88`); `Combatant` carries no faction
   (`crates/crpg-sim/src/combat.rs:207-236`), and `Combatant.tags` is the
   only group identity and nothing reads it (`crates/crpg-sim/src/combat.rs:224-227`).
   So `decide` takes `hostiles: &[EntityId]`; the host computes the set from
   faction `disposition` (`crates/crpg-data/src/document.rs:306-311`) and
   quest state. *Rejected:* infer hostility from `Combatant.tags` (tags are
   arbitrary author strings, not sides, and nothing defines "different tag
   ⇒ hostile"); from `placement` (the `direct_fight` test's proxy,
   `crates/crpg-cli/tests/play.rs:350-377`, is a fixture hack, not a side
   model); or add runtime sides to sim now (a persisted-state change, its
   own task, and the MVP has one hostile, `docs/CRPG_ENGINE_SPEC.md:1517`).

2. **The AI never mutates; it returns a decision the host applies.** The
   authority boundary is that clients cannot mutate authoritative state
   directly, and "options confer no authorization … host always rechecks
   execution" (`tasks/T028.md:20-30`). A read-only `decide(&World, ..)` keeps
   the decision separate from execution, so `perform_action` /
   `perform_move` / `set_route` revalidate whatever the AI proposes.
   *Rejected:* `decide(&mut World, ..)` that calls `perform_action` itself
   (mixes decision with execution and hides the revalidation from the host).

3. **"Nearest" is horizontal `seg_len`, ties broken by full `EntityId`.**
   Distance is `crpg_sim::seg_len(a, b)` — the horizontal
   `ceil_sqrt(dx² + dz²)` over raw values, `y` ignored
   (`crates/crpg-sim/src/movement.rs:310-315`) — the same measure `within_range`
   uses, so "nearest" and "in range" agree. A pair without a mover on both
   sides is unmeasurable and sorts after every measurable distance. The
   nearest is the minimum of `(distance.unwrap_or(u64::MAX), EntityId)`
   lexicographically, so equal distances break by ascending full `EntityId`
   (index then generation; `crates/crpg-core/src/entity.rs:60`), independent
   of `hostiles` order. *Rejected:* Euclidean-with-`y` (disagrees with the
   range measure); tie-break by slice order (non-deterministic across
   callers); `abs()`-based differences (overflows `i32::MIN`, which
   `seg_len` avoids).

4. **The move destination is the nearest hostile's own position.** "Move to
   nearest hostile" is literal: the AI paths to the hostile's `Mover::position()`
   (`crates/crpg-sim/src/movement.rs:116`), so once it arrives it is at
   distance 0 and in range next turn. *Rejected:* path to a point *just
   within* attack range (needs the range value, which the AI would have to
   re-derive instead of trusting `legal_actions`, and adds a step for no
   MVP gain); path to the target's cell centroid (also needs range, less
   deterministic).

5. **Only the nearest hostile is ever targeted, and the first affordable
   ability is the first legal one.** The AI attacks iff `legal_actions`
   lists a `UseAbility` whose `target` is the nearest hostile, and it picks
   the *first* such action in `legal_actions` order — abilities in ascending
   ULID × combatants in ascending `EntityId`
   (`crates/crpg-sim/src/combat.rs:3715-3717`) — which is exactly "the first
   affordable ability" (affordability is already inside `validate_action`,
   so an unaffordable ability is omitted). *Rejected:* scoring abilities
   (that is stage 2, `docs/CRPG_ENGINE_SPEC.md:700`, T080); attacking an
   in-range non-nearest hostile when the nearest is out of range (the spec
   says "nearest", and the MVP has one hostile so the fork is moot — kept
   as Open question 1).

6. **The AI is mode-agnostic; the closing action comes from `legal_actions`.** In
   TB `legal_actions` closes with `EndTurn { actor }`, in RT with
   `Stop { actor }` and never `EndTurn` (`crates/crpg-sim/src/combat.rs:3725-3727`).
   `decide` reads no mode field; it returns the closing action verbatim for
   `Pass`, and the host maps `Move` to `perform_move` (TB) or `set_route`
   (RT). *Rejected:* branching the AI on `EncounterMode`
   (`crates/crpg-sim/src/combat.rs:76-82`) — it duplicates what `legal_actions`
   already encodes and adds a second source of truth.

7. **The route is stamped with the bound revision and a constant tag.** The
   AI reads `world.nav()` (`crates/crpg-sim/src/world.rs:1228`) for the
   `MoveRoute.revision`, and `RouteTag(0)` for the tag (`RouteTag` is "the
   host's request identity … never interpreted", `crates/crpg-sim/src/movement.rs:62`).
   The host may overwrite the tag before applying. `nav` (the `NavMesh`
   argument) is used only for `find_path` geometry; the host guarantees the
   mesh's canonical bytes hash to `world.nav()` (sim never imports nav,
   `tasks/T036.md` Reading A2). *Rejected:* the AI computing the revision
   from the mesh (it has no `NavRevision::of_bytes` input and would
   duplicate the host's binding); a per-decision tag generator (host
   identity is not the AI's concern).

8. **Movement failures and a missing navmesh are `Pass`, not errors.** A
   `NavError` from `find_path` (`crates/crpg-nav/src/path.rs:39-58`), a
   hostile without a mover, or `world.nav() == None` all mean "I cannot move
   toward it this step", so the actor passes its turn. Only `legal_actions`
   failing is an `AiError`. *Rejected:* surfacing `NavError` to the host
   (the host has nothing to do about an unreachable target either, and the
   brief's "unreachable target" is a defined Pass, not a failure).

9. **`crpg-ai` stays integer-only and deterministic by contract.** The
   determinism lint scans exactly `CRATES = ["crpg-core", "crpg-rules",
   "crpg-sim", "crpg-nav"]` (`tools/lint/determinism.py:66`), so `crpg-ai` is
   not linted. The spec's agent-task constraint is "Deterministic: same world
   + seed → same decision" (`docs/CRPG_ENGINE_SPEC.md:1381`); this contract
   therefore requires, in `crates/crpg-ai/AGENTS.md`: no `f32`/`f64` or
   unsuffixed float literal, no `HashMap`/`HashSet` (use `BTreeMap`/`IndexMap`),
   no wall clock, no thread, no external RNG, no `unsafe` (already
   `#![forbid(unsafe_code)]`, `crates/crpg-ai/src/lib.rs:1`). `decide` draws
   no RNG and interns nothing, so determinism is a pure-function property.
   *Rejected:* relying on the lint alone (it does not scan the crate today;
   adding `crpg-ai` to it is Open question 3).

10. **Tests deserialize checked-in JSON worlds, because `crpg-ai` cannot
    dev-depend on `crpg-data` or `crpg-testkit`.** The ALLOWED row
    (`tools/lint/deps.py:53`) restricts every section, dev-dependencies
    included, to core/rules/sim/nav, and `start_encounter`/`register_creature`
    take `crpg-data`'s `Authored*` types (`crates/crpg-sim/src/combat.rs:882-897`,
    `crates/crpg-sim/src/creature.rs:118-127`). The implementer therefore
    materializes one TB and one RT world by running a crpg-sim-side helper
    once (a scratch example, not shipped), checks the JSON in, and the tests
    load it with `serde_json` and mutate only the fields a case needs (exactly
    as T036 edits MOVE through JSON, `tasks/T036.md:421-435`). *Rejected:*
    dev-depending on `crpg-data` or `crpg-testkit` (both violate `deps.py`);
    hand-building a `World` through public mutators (there is no public way
    to create a combatant outside `start_encounter`).

## Interface

Every public item has a doc comment (`#![warn(missing_docs)]`,
`crates/crpg-ai/src/lib.rs:2`). The whole public surface is three items;
`decide` is a pure function of its arguments.

```rust
//! MVP AI: move to the nearest hostile and attack (spec §6.1 Stage 1,
//! cut to the MVP scope by spec §17.3). Read-only and deterministic.

use crpg_core::EntityId;
use crpg_nav::NavMesh;
use crpg_sim::{CombatAction, LegalActionsError, MoveRoute, RouteTag, World};

/// What the MVP AI wants an actor to do next. The host applies it through
/// the existing public verbs, which revalidate (`Attack`/`Pass` via
/// `perform_action`, `Move` via `perform_move` in TB or `set_route` in RT).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Attack with this exact legal action — always a
    /// `CombatAction::UseAbility` targeting the nearest hostile
    /// (`crates/crpg-sim/src/combat.rs:932-952`).
    Attack(CombatAction),
    /// Walk this whole route toward the nearest hostile
    /// (`crates/crpg-sim/src/movement.rs:67-74`).
    Move(MoveRoute),
    /// Do nothing else; apply this closing action — `EndTurn { actor }` in
    /// turn-based, `Stop { actor }` in real time.
    Pass(CombatAction),
}

/// Why the AI could not produce a decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiError {
    /// The actor cannot act at all: `legal_actions` failed
    /// (`crates/crpg-sim/src/combat.rs:3678-3688`). Its text is the wrapped
    /// error's text; `source()` is the wrapped error.
    Legal(LegalActionsError),
}
// impl Display and std::error::Error for AiError, as LegalActionsError does
// (delegate to the inner error).

/// Choose one next step for `actor` (T056). Read-only and deterministic:
/// it draws no RNG, interns nothing, and mutates nothing.
///
/// * `nav` is the navmesh whose canonical bytes hash to `world.nav()`; the
///   host guarantees the match. It is consulted only when the AI decides to
///   move (which needs `world.nav() == Some`), so with no bound navmesh any
///   mesh may be passed and it is ignored.
/// * `hostiles` is the set of entities hostile to `actor`, in any order; the
///   result does not depend on that order. It must not contain `actor` (a
///   hostile equal to `actor` is a candidate only by host error).
pub fn decide(
    world: &World,
    actor: EntityId,
    nav: &NavMesh,
    hostiles: &[EntityId],
) -> Result<Decision, AiError>;
```

`crates/crpg-ai/Cargo.toml` `[dependencies]`: `crpg-core`, `crpg-sim`,
`crpg-nav` (workspace path deps). `[dev-dependencies]`: `serde_json` (for
the fixtures). No other dependency is added; `crpg-rules` is permitted but
unused.

## Behaviour

## Errors

## Tests

## What must not change

## Out of scope

## Open questions

## Agent log
