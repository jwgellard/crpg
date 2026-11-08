# crpg-nav architecture

## Purpose

`crpg-nav` is the pure, integer navigation crate: it validates convex
horizontal polygon meshes, will bake a box minus rectangular holes, will
answer `locate` and `nearest_on_mesh`, and will find paths with A* over
polygons and an integer funnel. Every returned segment must lie inside the
mesh (ADR-0036 V5). There is no float anywhere (V7), and nav knows nothing of
`crpg-sim`.

## Position in the graph

Depends on `crpg-core` only, for `Fx16_16` (re-exported so hosts name one
type). It is a sibling of `crpg-sim`, not a layer above or below it
(`tools/lint/deps.py` allows `crpg-nav -> crpg-core`). No external crate:
no `thiserror`, no `serde`, no `pathfinding`.

## Data flow (ADR-0036 A2)

Nav computes, the host supplies, sim steps. The host (T037, `crpg-server`)
bakes or loads a `NavMesh`, hashes its canonical bytes into a `NavRevision`,
asks `find_path`, and copies the returned points into sim's `MoveRoute`. Sim
never imports nav; nav never imports sim. The shared limits keep sim's values
under nav's names (`COORD_LIMIT_RAW`, `MAX_PATH_POINTS`,
`MAX_PATH_LENGTH_RAW`) so the host can assert equality at compile time.

## Modules

All modules are private; every public item is re-exported at the crate root.

- `lib.rs`: crate docs, the shared limits, the canonical-form constants
  (`NAV_MAGIC`, `NAV_FORMAT_VERSION`), `pub use` lines.
- `geom`: `ceil_sqrt` and `seg_len`, the horizontal U4 length. Squares are
  taken in `u128`, so no input overflows; `y` is ignored.
- `mesh`: `NavMesh`, `PolyId`, `NavMeshError`, `NavDecodeError`.
  - `NavMesh::new` validates in a fixed order: vertex count, polygon count,
    vertex bounds, duplicate vertices, then per polygon (size, vertex index,
    repeated vertex, height, convexity), then duplicate directed edges. The
    first failure is returned. Adjacency is derived: polygon `P`'s edge `i`
    neighbours `Q` exactly when `Q` holds the reversed vertex-index pair.
    A vertex-only touch is not adjacency.
  - Convexity is: every vertex on or left of every edge, collinear
    neighbours continue in the same direction, and at least one strict left
    turn. All cross products and dots are `i128` on raw values.
  - Canonical bytes: `CNAV`, version byte, `u32` vertex and polygon counts,
    `i32` little-endian `x, y, z` per vertex, then per polygon a `u8` count
    and `u32` indices. Adjacency is not stored. `from_canonical_bytes`
    rejects oversize counts before reading further, then runs `NavMesh::new`.

- `bake`: `bake_box(y, outer, holes)`, `NavRect`, `BakeError`,
  `MAX_BAKE_HOLES` (32). Errors are checked in declaration order: hole count,
  then any coordinate (height, outer, each hole) above `COORD_LIMIT_RAW`, then
  an empty outer, then per hole empty before outside. The bake is a full
  grid: the sorted distinct `x` and `z` edges of the outer rectangle and every
  hole cut the box into cells, and each cell no hole covers is one 4-vertex
  rectangle. Cells are emitted `x` ascending then `z` ascending, corners
  `(x0,z0) (x1,z0) (x1,z1) (x0,z1)`, and a vertex index is assigned on its
  first appearance in that order. Cells are never merged, so there is no
  T-junction and adjacency is exact. Holes may overlap and touch the outer
  edge, not cross it. No uncovered cell is `NoWalkableArea`.
- `query`: `NavMesh::locate` and `NavMesh::nearest_on_mesh`, all `i128`.
  - `locate` is the lowest polygon id whose every edge has `cross >= 0`
    (boundary included); `y` never matters.
  - `nearest_on_mesh` of a located point is the point at the polygon's `y`.
    Otherwise every polygon (id order) and edge (index order) yields a
    candidate and the first strictly nearest wins, so ties go to the lowest
    polygon id, then the lowest edge index.
  - The edge candidate projects `p` on the edge with `t = (p - a) . e` and
    `l2 = e . e`: `a` if `t <= 0`, `b` if `t >= l2`; otherwise the foot's
    floor and ceiling in each axis (`div_euclid`, a floor for negatives too)
    give four lattice points tried in the order `(fx,fz) (fx,cz) (cx,fz)
    (cx,cz)`; the nearest contained one wins, the first on a tie. If none is
    contained the nearer end wins (`a` on a tie), so `locate` of the answer is
    always `Some`.
- `path`: `NavMesh::find_path`, `NavPath`, `NavError`.
  - Errors in order: `StartOffMesh`, `GoalOffMesh`, `AlreadyThere` (`x`/`z`
    only), then `Unreachable`, `PathTooComplex` (more than 64 points) and
    `TooLong` (more than `MAX_PATH_LENGTH_RAW`). The same polygon is the
    corridor `[s]`.
  - A* is pinned. Nodes are polygons; a node's point is `from` for the start,
    the goal for the goal polygon, else the floor centroid of its vertices
    (`i64` sums, `div_euclid`). Edge cost and heuristic are `seg_len`. The open
    set is a `BTreeSet<(f, h, id)>` popped least first; a popped node is closed
    and never reopens; a neighbour changes only on a strict cost improvement
    (its old open entry is removed first); neighbours go in edge order.
  - Portals: for each corridor pair the lowest edge index of the polygon being
    left that neighbours the next; `left` is that edge's end vertex, `right` its
    start; `(from, from)` first and `(goal, goal)` last.
  - The funnel is the "simple stupid funnel" with `i128` cross products, as
    written in B-P5: the apex restarts at the portal after the chosen corner.
    Points are `from`, then each corner, then the goal, dropping any point
    `same` (`x`/`z`) as the one before. `from` keeps its `y`, a corner takes its
    vertex's `y`, the goal takes the goal polygon's `y`.
  - V5 oracle (used by `tests/path.rs`): a segment `(q, r)` is clear of a hole
    `[x0,z0,x1,z1]` when `max(q.x, r.x) <= x0`, `min(q.x, r.x) >= x1`, or the
    same on `z`, or the four hole corners are all on or left of `q -> r` or all
    on or right of it. Shortest paths are not promised.

## What exists and what is planned

Exists (T035a, T035b, T035c): `geom`, `mesh`, `bake`, `query`, `path`, the
limits and the canonical form. Out of scope for this crate: baking from
`Area.bounds`, `NavRevision`, `MoveRoute` (T037, T036).

## Governing decisions

[ADR-0036](../adr/0036-movement-and-navigation.md) (V1-V7, U3, U4) and D27c;
the task contract is [T035](../../tasks/T035.md). Consumers: T037 (host
integration), T026, T050.

## Agent log

- 2026-11-08 (UTC) · claude-code/sonnet-5.5 (implementer) + T035a · Opened the doc with the crate's place in the graph, the nav-host-sim data flow, and the as-built geometry, mesh validation and canonical bytes, leaving the bake, queries and paths as T035b and T035c.

- 2026-11-08 (UTC) · claude-code/sonnet-5.5 (implementer) + T035b · Added the as-built `bake` grid rule and the `locate` / `nearest_on_mesh` tie-breaks and four-candidate rounding rule, and moved the bake and queries from planned to existing.

- 2026-11-08 (UTC) · claude-code/sonnet-5.5 (implementer) + T035c · Added the as-built `path` module (pinned A* order, portals, the integer funnel, error order) and the V5 segment-versus-hole oracle, and moved paths from planned to existing.
