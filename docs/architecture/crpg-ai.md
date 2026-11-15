# crpg-ai architecture

## Purpose

`crpg-ai` is the engine's first AI: a pure, read-only decision for one creature
the server controls inside an encounter. For one actor it says what to do next
— attack the nearest hostile it may legally attack, walk toward the nearest
hostile, or end the turn / hold — and nothing else. It never mutates the world,
never executes an action and never calls a host operation; the host driver
applies each `Decision` through its own trusted operations (T037a's
`host_move`, `move_to` and `halt`, and the ordinary action path), each of which
revalidates.

Today that is the MVP of spec §6.1 stage 1, cut to "move to the nearest hostile
and attack" (D27f, `docs/CRPG_ENGINE_SPEC.md` §17.3). Utility scoring, AI profiles
as data, behaviour trees, party AI, the encounter dry-run simulator and ambient
(aggro radius, line-of-sight) AI are later tasks.

## Position in the graph

Depends on `crpg-core` (identity and fixed-point), `crpg-sim` (the one legality
source, movement arithmetic and world state) and `crpg-nav` (the host's mesh,
its queries and paths). No external crate, no dev-dependency; `crpg-rules` is
allowed by `tools/lint/deps.py` but not used. Sim, nav and rules never depend on
this crate, so the AI is a leaf consumer of the simulation surface.

Hostility is not in that surface: sim, rules and core have no sides, teams or
allegiance, and `crpg-ai` may not read `crpg-data` where factions are authored.
The host driver therefore builds a `Sides` table from authored data and passes
it in (`tasks/T056.md` Reading 1).

## Data flow

The host owns everything the AI only reads:

1. The host starts the encounter and moves the actors; the chosen legal action
   is one of `legal_actions`' options and the destination is a point on the
   host's own mesh.
2. Each turn (turn-based) or tick (real time) the host calls `decide(world,
   actor, sides, mesh)` and maps the `Decision`: `Act` to the action path,
   `Move` to `host_move` or `move_to`, `Halt` to `halt`, `Hold` to nothing.
3. The AI's move is a *destination*; the host recomputes the route with
   `plan_route`/`host_route`, the same computation the AI used to check it, so
   both sides agree on the route the mesh yields.

`decide` returns `Result<Decision, LegalActionsError>`: an actor that cannot act
is the caller's to handle, and a movement or navigation failure is an ordinary
`Decision`, never an error.

## Modules

All modules are private; every public item is re-exported at the crate root.

- `lib.rs`: crate docs and the `pub use` lines.
- `sides`: `SideId`, `Sides`, `SidesError`. `Sides::new` sorts the entries by
  ascending `EntityId` and reports the least entity listed twice; `side` is a
  binary search; `hostile(a, b)` is both listed and different (so never
  `hostile(e, e)`). An unlisted entity is hostile to no one.
- `route`: `host_route`, `point_along`, `RouteError`, `AI_ROUTE_TAG`,
  `STANDOFF_MARGIN_RAW`, and the crate-private `measure_of`.
  - `host_route` is T037 A-5's computation: a bound revision, the actor, its
    mover, then `nearest_on_mesh(snap(measure, destination))` and
    `find_path(mover.position(), goal)`, copied whole into a `MoveRoute` tagged
    `AI_ROUTE_TAG` (0, the host's tag). Refusals come in that order.
  - `point_along` walks a polyline by U4 segment length with sim's `seg_len` and
    `point_at`, skipping zero-length segments; at or beyond the total it is the
    last point.
- `mvp`: `Decision` and `decide`, plus the private `distance`, `pool_balance`
  and `reserved_cost`. `decide` is M-1..M-9 of `tasks/T056.md`:
  1. `legal_actions` first; its error is returned unchanged.
  2. Hostiles are the live, non-self combatants on another side, ascending
     `EntityId`; with none, turn-based ends the turn and real time stops a
     standing order, else halts a route, else holds.
  3. Otherwise it takes the least listed `UseAbility` against a hostile by
     `(seg_len to the target, target, ability)` — turn-based acts at once; real
     time halts a route first, holds when that exact order already stands, and
     otherwise issues the action.
  4. With no attack available it walks: the approach target is the nearest
     hostile, the intended ability the least ULID whose use fails exactly
     `OutOfRange`, and the destination the point whose remaining path length is
     `range − STANDOFF_MARGIN_RAW`, capped in turn-based play by the movement
     pool's balance less that ability's cost in the same pool, in whole steps.
  5. The move is accepted only if `host_route` and `validate_move` (turn-based)
     or `validate_route` (real time) accept it; every other outcome is one of
     the defined fallbacks.
  6. In real time a route that already ends within range holds without a path
     query, so the mesh may be absent.

## What exists and what is planned

Exists (T056a): `Sides`, `host_route`, `point_along`, `decide`. Tests in this
crate cover the helpers only (U1–U8); the decision's M-1..M-9 need a started
encounter, which needs `crpg-data`, so they are pinned by `crpg-testkit`'s
encounter suite (T056b).

Planned: T080 utility scoring, T081 AI profiles as data, T082 behaviour trees,
T083 party AI, T084 the encounter dry-run simulator, and the host driver
(`crpg-server`) that builds `Sides`, calls `decide` on turn start and on a
real-time cadence, caps calls per turn and maps the `Decision`.

## Governing decisions

Spec §6 (AI) and §6.1 stage 1; §17.3 with D27f for the MVP cut; ADR-0036 for
the movement and navigation arithmetic the decision consumes; the contract is
[T056](../../tasks/T056.md), approved by D63.

## Consumers

`crpg-server` (the host driver, later task) and `crpg-testkit` (the T056b
behaviour suite). Nothing below the host may depend on this crate.

## Agent log

- 2026-11-15 (UTC) · opencode-go/deepseek-v4.1-flash + T056a · Opened the doc with the crate's place in the graph, the caller-supplied `Sides` boundary, the host-owns-the-route data flow, the three modules as built, and the M-1..M-9 decision order.
