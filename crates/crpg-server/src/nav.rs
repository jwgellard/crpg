//! Navigation binding and the one route computation (T037a; ADR-0036 H1, H2,
//! H5, S3).
//!
//! The host owns the navigation mesh: [`HostNav`] pairs a validated
//! `crpg_nav::NavMesh` with the revision sim validates routes against.
//! [`area_mesh`] is H1's bake from an area's two bounds, [`measure_of`] picks
//! the encounter's movement measure, and [`plan_route`] is the one function
//! behind every route request: the mover's position, S3's snap, the nearest
//! mesh point as the goal, then the mesh's path (Reading 6). Sim never
//! imports the navigation crate (A2), so this module belongs to the host.
//!
//! The mesh is host state, not checkpointed (Reading 3): a restored host has
//! no mesh until the embedding binds one again.

use crpg_core::{EntityId, Fx16_16};
use crpg_sim::{snap, Measure, MoveError, MoveRoute, NavRevision, RouteTag, World};

use crpg_nav::NavMesh;

// ---------------------------------------------------------------------------
// H5: the limits are shared at compile time.
// ---------------------------------------------------------------------------

/// The navigation and simulation position bounds are one bound.
const _: () = assert!(crpg_nav::COORD_LIMIT_RAW == crpg_sim::COORD_LIMIT_RAW);
/// A nav path and a sim route hold the same number of points.
const _: () = assert!(crpg_nav::MAX_PATH_POINTS == crpg_sim::MAX_ROUTE_POINTS);
/// A nav path and a sim route share their length bound.
const _: () = assert!(crpg_nav::MAX_PATH_LENGTH_RAW == crpg_sim::MAX_ROUTE_LENGTH_RAW);
/// The v4 wire's position bound is the simulation's.
const _: () = assert!(crpg_net::protocol_v4::COORD_LIMIT_RAW == crpg_sim::COORD_LIMIT_RAW);
/// The v4 wire's moved-point bound is the sim route's.
const _: () = assert!(crpg_net::protocol_v4::MAX_MOVED_POINTS == crpg_sim::MAX_ROUTE_POINTS);

// ---------------------------------------------------------------------------
// Host state.
// ---------------------------------------------------------------------------

/// The bound navigation mesh and the revision routes are validated against
/// (H1): host state, never checkpointed (Reading 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HostNav {
    /// The bound mesh.
    pub(crate) mesh: NavMesh,
    /// BLAKE3 of the mesh's canonical bytes (R2).
    pub(crate) revision: NavRevision,
}

/// Why [`plan_route`] returned no route; first failure wins (A-5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RouteRefusal {
    /// No mesh is bound to this host.
    NoNavMesh,
    /// The sim refused the request's subject.
    Move(MoveError),
    /// Navigation found no path.
    Nav(crpg_nav::NavError),
}

// ---------------------------------------------------------------------------
// Baking and measuring.
// ---------------------------------------------------------------------------

/// Reading 2: `bake_box(min[1], NavRect { min[0], min[2], max[0], max[2] }, &[])`.
///
/// The floor is the bounds' lowest `y` (H1's v1 bake).
pub(crate) fn area_mesh(
    min: [Fx16_16; 3],
    max: [Fx16_16; 3],
) -> Result<NavMesh, crpg_nav::BakeError> {
    crpg_nav::bake_box(
        min[1],
        crpg_nav::NavRect {
            min_x: min[0],
            min_z: min[2],
            max_x: max[0],
            max_z: max[2],
        },
        &[],
    )
}

/// Reading 6: the encounter's `TurnMovement.measure`, else `Measure::Euclidean`.
pub(crate) fn measure_of(world: &World) -> Measure {
    world
        .combat()
        .and_then(|combat| combat.definition.movement)
        .map_or(Measure::Euclidean, |movement| movement.measure)
}

// ---------------------------------------------------------------------------
// The one route computation (H2's path, S3's snap).
// ---------------------------------------------------------------------------

/// A-5: the route from `actor`'s position to `destination`, tagged `tag`.
///
/// Reads only; checks in order: a bound mesh (`NoNavMesh`), the actor's
/// presence (`Move(AbsentEntity)`), its mover (`Move(NotMover)`), then the
/// mesh's path from the mover's exact position to
/// `nearest_on_mesh(snap(measure, destination))` (`Nav`). The path's points
/// are copied whole into the route (A2, R5).
pub(crate) fn plan_route(
    nav: Option<&HostNav>,
    world: &World,
    actor: EntityId,
    destination: [Fx16_16; 3],
    tag: RouteTag,
) -> Result<MoveRoute, RouteRefusal> {
    let Some(nav) = nav else {
        return Err(RouteRefusal::NoNavMesh);
    };
    if !world.contains(actor) {
        return Err(RouteRefusal::Move(MoveError::AbsentEntity {
            entity: actor,
        }));
    }
    let Some(mover) = world.movers().get(actor) else {
        return Err(RouteRefusal::Move(MoveError::NotMover { entity: actor }));
    };
    let goal = nav
        .mesh
        .nearest_on_mesh(snap(measure_of(world), destination));
    let path = nav
        .mesh
        .find_path(mover.position(), goal)
        .map_err(RouteRefusal::Nav)?;
    Ok(MoveRoute {
        revision: nav.revision,
        tag,
        points: path.into_points(),
    })
}
