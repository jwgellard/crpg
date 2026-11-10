//! Host-fed per-field event projection for protocol v3 (T140b, ADR-0029 N1).
//!
//! The same contract as [`crate::projection_v2`]: each `Some` value means the
//! host has independently authorized disclosure of that field for this viewer,
//! and `None` (or a `false` grant) means forbidden or identity unavailable. A
//! candidate missing any required field or grant is omitted whole, without
//! inspecting its other fields. Projection is stateless: it mints no ids,
//! reads no `World`, and holds no history.
//!
//! [`project_events`] visits candidates in input order and appends exactly one
//! op per fully disclosed event: never sorted, never deduplicated.
//! [`EventCandidate::V2`] wraps a v2 candidate and yields exactly the ops
//! [`crate::projection_v2::project_events`] returns for it, converted with
//! `From`; its errors fail the whole call.

use crate::protocol::MAX_DELTA_OPS;
use crate::protocol_v3::{DeltaOp, EncounterMode, NetId};

pub use crate::projection_v2::ProjectionError;

/// One host-authorized v3 projection input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventCandidate {
    /// A v2 candidate (spawn, despawn, death, resolution, turn start,
    /// encounter end), projected as v2 does and converted into v3 ops.
    V2(crate::projection_v2::EventCandidate),
    /// A round start needs the round fact.
    RoundStarted {
        /// The disclosable round, or `None` when withheld.
        round: Option<u64>,
    },
    /// A mode change needs the new mode and the round.
    ModeChanged {
        /// The disclosable new mode, or `None` when withheld.
        mode: Option<EncounterMode>,
        /// The disclosable round, or `None` when withheld.
        round: Option<u64>,
    },
    /// An ended standing order needs the actor mapping (granted only to the
    /// controlling peer by the host).
    OrderEnded {
        /// The disclosable actor, or `None` when unmapped or hidden.
        actor: Option<NetId>,
    },
    /// A join notice needs the entity mapping and the grant.
    Joined {
        /// The disclosable entity, or `None` when unmapped or hidden.
        entity: Option<NetId>,
        /// The explicit join-notice grant for this viewer.
        disclose: bool,
    },
    /// A leave notice needs the entity mapping and the grant.
    Left {
        /// The disclosable entity, or `None` when unmapped or hidden.
        entity: Option<NetId>,
        /// The explicit leave-notice grant for this viewer.
        disclose: bool,
    },
}

/// Projects host-authorized candidates into ordered v3 delta ops.
///
/// More than [`MAX_DELTA_OPS`] candidates is
/// [`ProjectionError::TooManyEvents`] with no output. Otherwise candidates are
/// visited in input order and one op is appended per fully disclosed event.
/// `RoundStarted` needs `round`, `ModeChanged` needs `mode` and `round`,
/// `OrderEnded` needs `actor`, and `Joined`/`Left` need `entity` and
/// `disclose == true`; a candidate missing any of these is omitted without
/// inspecting its other fields. Any v2 error discards the whole page.
pub fn project_events(candidates: &[EventCandidate]) -> Result<Vec<DeltaOp>, ProjectionError> {
    if candidates.len() > MAX_DELTA_OPS {
        return Err(ProjectionError::TooManyEvents);
    }
    let mut ops = Vec::new();
    for candidate in candidates {
        match candidate {
            EventCandidate::V2(inner) => {
                let projected = crate::projection_v2::project_events(core::slice::from_ref(inner))?;
                ops.extend(projected.into_iter().map(DeltaOp::from));
            }
            EventCandidate::RoundStarted { round } => {
                let Some(round) = round else {
                    continue;
                };
                ops.push(DeltaOp::RoundStarted { round: *round });
            }
            EventCandidate::ModeChanged { mode, round } => {
                let (Some(mode), Some(round)) = (mode, round) else {
                    continue;
                };
                ops.push(DeltaOp::ModeChanged {
                    mode: *mode,
                    round: *round,
                });
            }
            EventCandidate::OrderEnded { actor } => {
                let Some(actor) = actor else {
                    continue;
                };
                ops.push(DeltaOp::OrderEnded { actor: *actor });
            }
            EventCandidate::Joined { entity, disclose } => {
                let (Some(entity), true) = (entity, disclose) else {
                    continue;
                };
                ops.push(DeltaOp::Joined { entity: *entity });
            }
            EventCandidate::Left { entity, disclose } => {
                let (Some(entity), true) = (entity, disclose) else {
                    continue;
                };
                ops.push(DeltaOp::Left { entity: *entity });
            }
        }
    }
    Ok(ops)
}
