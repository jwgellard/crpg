//! Host-fed per-field history projection (T021, ADR-0019).
//!
//! Production consumes host-supplied [`EventCandidate`]s, never raw sim
//! events: each `Some` value means the host has independently authorized
//! disclosure of **that field** for this viewer and generation, while
//! `None` means forbidden or identity unavailable. Unknown grants are
//! `None`/`false`. These plain values are not authentication credentials,
//! and ordinary clients never construct authoritative projection inputs:
//! T022 maps actual ordered history envelopes into candidates in its test
//! adapter, and T027b/c later supply production interest facts and grants.
//!
//! Disclosure contract (one row per history fact; any absent required
//! field or grant omits the whole event, without inspecting hidden payload
//! values further):
//!
//! - `Spawned`/`Despawned`/`Died`: the entity mapping plus the explicit
//!   corresponding cause grant.
//! - `ActionResolved`: both entity mappings, the authored ability id, the
//!   symbolic outcome, and the reported damage.
//! - `TurnStarted`: the actor mapping plus the round/turn-start fact.
//! - `EncounterEnded`: the authored encounter identity plus the terminal
//!   round/end fact.
//!
//! [`project_events`] visits candidates in input order and appends exactly
//! one op per fully disclosed event, so output is an ordered subsequence:
//! never sorted by actor, tag, or [`NetId`](crate::protocol::NetId), never
//! deduplicated. Same-tick and repeated logical turns keep journal order.
//! Independently permitted state ops are assembled by the host, so
//! suppressing an event never suppresses a Health/Turn update, and an
//! entity leave for nondisclosable removal is a separate host
//! state-membership decision, not an inferred event.
//!
//! Projection is stateless: no reader/ack, id minting, live replica, or
//! history retention here. The host assigns per-client frame `event_seq`
//! after suppression and chunking, starting at 1, and never reuses a
//! history sequence. A suppressed-only page produces no frame and consumes
//! no delivery sequence.

use core::fmt;

use crpg_core::Ulid;

use crate::protocol::{NetId, MAX_DELTA_OPS, MAX_WIRE_STRING_BYTES};
use crate::protocol_v2::DeltaOp;

/// One host-authorized projection input: per-field optional values plus
/// explicit disclosure grants.
///
/// T022 maps real history envelopes into these in its test adapter; the
/// production host supplies them from its own interest facts. Never filled
/// with zero or placeholder values for hidden fields: absence is `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventCandidate {
    /// A permitted spawn notice needs the entity mapping and the grant.
    Spawned {
        /// The disclosable replica entity, or `None` when unmapped/hidden.
        entity: Option<NetId>,
        /// The explicit spawn-cause grant for this viewer/generation.
        disclose: bool,
    },
    /// A permitted despawn notice needs the entity mapping and the grant.
    Despawned {
        /// The disclosable replica entity, or `None` when unmapped/hidden.
        entity: Option<NetId>,
        /// The explicit despawn-cause grant for this viewer/generation.
        disclose: bool,
    },
    /// A permitted death notice needs the entity mapping and the grant.
    Died {
        /// The disclosable replica entity, or `None` when unmapped/hidden.
        entity: Option<NetId>,
        /// The explicit death-cause grant for this viewer/generation.
        disclose: bool,
    },
    /// A permitted resolution needs every field: both mappings, the
    /// authored ability id, the symbolic outcome, and the reported damage.
    ActionResolved {
        /// The disclosable acting replica entity.
        actor: Option<NetId>,
        /// The disclosable targeted replica entity.
        target: Option<NetId>,
        /// The authored ability identity.
        ability: Option<Ulid>,
        /// The symbolic outcome text.
        outcome: Option<String>,
        /// The reported flat damage.
        damage: Option<u32>,
    },
    /// A permitted turn start needs the actor mapping and the round fact.
    TurnStarted {
        /// The disclosable incoming turn head.
        actor: Option<NetId>,
        /// The disclosable round/turn-start fact.
        round: Option<u64>,
    },
    /// A permitted natural end needs the encounter identity and the fact.
    EncounterEnded {
        /// The authored encounter identity.
        encounter: Option<Ulid>,
        /// The disclosable terminal round/end fact.
        round: Option<u64>,
    },
}

/// Projection failure: input bound, outcome vocabulary, or string bound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectionError {
    /// More than [`MAX_DELTA_OPS`](crate::protocol::MAX_DELTA_OPS) input
    /// candidates were supplied.
    TooManyEvents,
    /// A disclosed outcome is outside T020's symbolic vocabulary.
    InvalidOutcome,
    /// A disclosed outcome exceeds the wire string byte bound.
    StringLimit,
}

impl fmt::Display for ProjectionError {
    /// Renders `<VariantName> at projection/events`, without payloads.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyEvents => write!(f, "TooManyEvents at projection/events"),
            Self::InvalidOutcome => write!(f, "InvalidOutcome at projection/events"),
            Self::StringLimit => write!(f, "StringLimit at projection/events"),
        }
    }
}

impl std::error::Error for ProjectionError {}

/// Projects host-authorized candidates into ordered v2 delta ops.
///
/// Checks at most [`MAX_DELTA_OPS`](crate::protocol::MAX_DELTA_OPS) input
/// candidates ([`ProjectionError::TooManyEvents`] beyond that), then visits
/// in input order. Any event missing a required field or grant is omitted
/// whole, without inspecting its hidden payload values further; otherwise
/// the outcome byte bound ([`ProjectionError::StringLimit`]) then symbolic
/// validity ([`ProjectionError::InvalidOutcome`]) are checked and exactly
/// one op is appended. Returns no partial output on error: any failure
/// discards the whole page. Structural notices project through
/// [`DeltaOp::Legacy`] with their v1 tag and payload.
pub fn project_events(candidates: &[EventCandidate]) -> Result<Vec<DeltaOp>, ProjectionError> {
    if candidates.len() > MAX_DELTA_OPS {
        return Err(ProjectionError::TooManyEvents);
    }
    let mut ops = Vec::new();
    for candidate in candidates {
        match candidate {
            EventCandidate::Spawned { entity, disclose } => {
                let (Some(entity), true) = (entity, disclose) else {
                    continue;
                };
                ops.push(DeltaOp::Legacy(crate::protocol::DeltaOp::Spawned {
                    entity: *entity,
                }));
            }
            EventCandidate::Despawned { entity, disclose } => {
                let (Some(entity), true) = (entity, disclose) else {
                    continue;
                };
                ops.push(DeltaOp::Legacy(crate::protocol::DeltaOp::Despawned {
                    entity: *entity,
                }));
            }
            EventCandidate::Died { entity, disclose } => {
                let (Some(entity), true) = (entity, disclose) else {
                    continue;
                };
                ops.push(DeltaOp::Legacy(crate::protocol::DeltaOp::Died {
                    entity: *entity,
                }));
            }
            EventCandidate::ActionResolved {
                actor,
                target,
                ability,
                outcome,
                damage,
            } => {
                let (Some(actor), Some(target), Some(ability), Some(outcome), Some(damage)) =
                    (actor, target, ability, outcome, damage)
                else {
                    continue;
                };
                if outcome.len() > MAX_WIRE_STRING_BYTES {
                    return Err(ProjectionError::StringLimit);
                }
                if !crate::codec_v2::valid_outcome(outcome) {
                    return Err(ProjectionError::InvalidOutcome);
                }
                ops.push(DeltaOp::ActionResolved {
                    actor: *actor,
                    target: *target,
                    ability: *ability,
                    outcome: outcome.clone(),
                    damage: *damage,
                });
            }
            EventCandidate::TurnStarted { actor, round } => {
                let (Some(actor), Some(round)) = (actor, round) else {
                    continue;
                };
                ops.push(DeltaOp::TurnStarted {
                    actor: *actor,
                    round: *round,
                });
            }
            EventCandidate::EncounterEnded { encounter, round } => {
                let (Some(encounter), Some(round)) = (encounter, round) else {
                    continue;
                };
                ops.push(DeltaOp::EncounterEnded {
                    encounter: *encounter,
                    round: *round,
                });
            }
        }
    }
    Ok(ops)
}
