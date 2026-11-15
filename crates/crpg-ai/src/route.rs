//! The host's route computation and the standoff point (T056a; T037 A-5).
//!
//! [`host_route`] is the same computation T037a's `plan_route` performs: the
//! mover's exact position, S3's snap of the destination, the nearest mesh
//! point as the goal, then the mesh's path copied whole. The AI hands the host
//! a destination, the host recomputes the route, and both sides see the same
//! integer answer.

use std::fmt;

use crpg_core::{EntityId, Fx16_16};
use crpg_sim::{point_at, seg_len, snap, Measure, MoveRoute, RouteTag, World};

/// The tag of every route `host_route` builds; equal to T037a's
/// `HOST_ROUTE_TAG`. Never interpreted by sim.
pub const AI_ROUTE_TAG: RouteTag = RouteTag(0);

/// How far inside an ability's range the approach stops, raw (Reading 7).
pub const STANDOFF_MARGIN_RAW: u64 = 64;

/// Why `host_route` built no route (T037 A-5's refusals, in its order).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteError {
    /// The world has no bound navmesh revision.
    NavUnbound,
    /// The actor is not live.
    AbsentEntity {
        /// The absent entity.
        entity: EntityId,
    },
    /// The live actor has no mover.
    NotMover {
        /// The entity without a mover.
        entity: EntityId,
    },
    /// Navigation found no path.
    Nav(crpg_nav::NavError),
}

impl fmt::Display for RouteError {
    /// Renders `<Variant> at ai/route`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NavUnbound => write!(f, "NavUnbound at ai/route"),
            Self::AbsentEntity { .. } => write!(f, "AbsentEntity at ai/route"),
            Self::NotMover { .. } => write!(f, "NotMover at ai/route"),
            Self::Nav(_) => write!(f, "Nav at ai/route"),
        }
    }
}

impl std::error::Error for RouteError {
    /// Nothing is wrapped as a source (the nav error is a value, not a cause).
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        None
    }
}

/// The encounter's `TurnMovement.measure`, else `Measure::Euclidean`.
///
/// Exactly `crates/crpg-sim/src/combat.rs`'s range-stage choice, shared by
/// [`host_route`] and the decision.
pub(crate) fn measure_of(world: &World) -> Measure {
    world
        .combat()
        .and_then(|state| state.definition.movement)
        .map_or(Measure::Euclidean, |movement| movement.measure)
}

/// The route the host computes for `destination` (T037 A-5, Reading 4).
///
/// Reads only. Checks in order, the first failure winning: a bound revision
/// ([`RouteError::NavUnbound`]), the actor's presence
/// ([`RouteError::AbsentEntity`]), its mover ([`RouteError::NotMover`]), then
/// the mesh's path from the mover's exact position to
/// `nearest_on_mesh(snap(measure, destination))` ([`RouteError::Nav`]).
pub fn host_route(
    nav: &crpg_nav::NavMesh,
    world: &World,
    actor: EntityId,
    destination: [Fx16_16; 3],
) -> Result<MoveRoute, RouteError> {
    let Some(revision) = world.nav() else {
        return Err(RouteError::NavUnbound);
    };
    if !world.contains(actor) {
        return Err(RouteError::AbsentEntity { entity: actor });
    }
    let Some(mover) = world.movers().get(actor) else {
        return Err(RouteError::NotMover { entity: actor });
    };
    let goal = nav.nearest_on_mesh(snap(measure_of(world), destination));
    let path = nav
        .find_path(mover.position(), goal)
        .map_err(RouteError::Nav)?;
    Ok(MoveRoute {
        revision,
        tag: AI_ROUTE_TAG,
        points: path.into_points(),
    })
}

/// The point `distance` raw U4 length along a polyline (R-2).
///
/// `None` for an empty slice; the only point for one point. A zero-length
/// segment is skipped; a `distance` at or beyond the total gives the last
/// point.
pub fn point_along(points: &[[Fx16_16; 3]], distance: u64) -> Option<[Fx16_16; 3]> {
    points.first()?;
    let mut remaining = distance;
    for pair in points.windows(2) {
        let length = seg_len(pair[0], pair[1]);
        if length == 0 {
            continue;
        }
        if remaining < length {
            return Some(point_at(pair[0], pair[1], length, remaining));
        }
        remaining -= length;
    }
    points.last().copied()
}
