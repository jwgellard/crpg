//! Host-fed event projection for protocol v4 (T026p3, ADR-0036 L1).
//!
//! The same contract as [`crate::projection_v3`]: a candidate missing a
//! required field or grant is omitted whole. [`EventCandidate::V3`] wraps a v3
//! candidate and yields exactly the ops
//! [`crate::projection_v3::project_events`] returns for it, wrapped in
//! [`DeltaOp::V3`]. [`EventCandidate::Moved`] is disclosed under the viewer's
//! position grant through the host-set `disclose` flag, as `Joined`/`Left`
//! are. Projection is stateless: it mints no ids and reads no `World`.

use crate::protocol::MAX_DELTA_OPS;
use crate::protocol_v4::{check_points, DeltaOp, NetId, Point};

pub use crate::projection_v2::ProjectionError;

/// One host-authorized v4 projection input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventCandidate {
    /// A v3 candidate, projected as v3 does and wrapped in [`DeltaOp::V3`].
    V3(crate::projection_v3::EventCandidate),
    /// A turn-based move needs the entity mapping, the grant and valid
    /// points.
    Moved {
        /// The disclosable entity, or `None` when unmapped or hidden.
        entity: Option<NetId>,
        /// The route's points; checked by `check_points` when disclosed.
        points: Vec<Point>,
        /// The explicit position grant for this viewer.
        disclose: bool,
    },
}

/// Projects host-authorized candidates into ordered v4 delta ops.
///
/// More than [`MAX_DELTA_OPS`] candidates is
/// [`ProjectionError::TooManyEvents`] with no output. Otherwise candidates are
/// visited in input order and one op is appended per disclosed event: no
/// sorting, no deduplication. `Moved` needs `entity`, `disclose == true` and
/// points that pass `check_points`; a candidate failing any of these is
/// omitted. Any v3 error discards the whole page.
pub fn project_events(candidates: &[EventCandidate]) -> Result<Vec<DeltaOp>, ProjectionError> {
    if candidates.len() > MAX_DELTA_OPS {
        return Err(ProjectionError::TooManyEvents);
    }
    let mut ops = Vec::new();
    for candidate in candidates {
        match candidate {
            EventCandidate::V3(inner) => {
                let projected = crate::projection_v3::project_events(core::slice::from_ref(inner))?;
                ops.extend(projected.into_iter().map(DeltaOp::from));
            }
            EventCandidate::Moved {
                entity,
                points,
                disclose,
            } => {
                let (Some(entity), true) = (entity, disclose) else {
                    continue;
                };
                if check_points(points).is_err() {
                    continue;
                }
                ops.push(DeltaOp::Moved {
                    entity: *entity,
                    points: points.clone(),
                });
            }
        }
    }
    Ok(ops)
}
