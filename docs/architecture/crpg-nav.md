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

## What exists and what is planned

Exists (T035a): `geom`, `mesh`, the limits and the canonical form.
Planned: T035b adds `bake_box` (a full grid of unmerged rectangles),
`locate` and `nearest_on_mesh`; T035c adds `find_path` and `NavPath` (pinned
A* and the integer funnel). Out of scope for this crate: baking from
`Area.bounds`, `NavRevision`, `MoveRoute` (T037, T036).

## Governing decisions

[ADR-0036](../adr/0036-movement-and-navigation.md) (V1-V7, U3, U4) and D27c;
the task contract is [T035](../../tasks/T035.md). Consumers: T037 (host
integration), T026, T050.

## Agent log

- 2026-11-08 (UTC) · claude-code/sonnet-5.5 (implementer) + T035a · Opened the doc with the crate's place in the graph, the nav-host-sim data flow, and the as-built geometry, mesh validation and canonical bytes, leaving the bake, queries and paths as T035b and T035c.
