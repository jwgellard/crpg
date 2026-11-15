//! Engine-neutral client replica of one filtered v4 session (T026p5,
//! ADR-0036 L4–L5).
//!
//! [`ReplicaWorldV4`] is the v3 replica plus positions and samples. It wraps a
//! [`ReplicaWorldV3`] and applies every `V3` op through that replica's
//! `apply_op`, so each v3 rule stays one implementation; the v4 state is the
//! disclosed positions and the newest sample's clock. `Moved` is an unchecked
//! notice (like `Joined`/`Left`): the replica stores nothing from it. A
//! position comes only from a `Position` anchor or a sample, and the newest
//! tick wins.
//!
//! It reads no `World`, clock, RNG, I/O or configuration, mints nothing and
//! derives no presence or visibility. Every refusal is an existing
//! [`ReplicaError`] and leaves the replica unchanged; [`ReplicaWorldV4::apply_sample`]
//! never fails, because a datagram that is stale, duplicated or about an
//! entity the replica does not know is ordinary traffic, not a fault.

use std::collections::BTreeMap;

use crate::movement_v4::MoveSample;
use crate::pool_v3::PublicationV3;
use crate::protocol::{LANE_COMBAT, MAX_DELTA_OPS, SEQ_LAST};
use crate::protocol_v3::DeltaFrame as DeltaFrameV3;
use crate::protocol_v3::DeltaOp as DeltaOpV3;
use crate::protocol_v4::{DeltaFrame, DeltaOp, NetId, Point, SessionEpoch};
use crate::replica_v3::ReplicaWorldV3;

pub use crate::replica::ReplicaError;

/// One disclosed position as last received.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PositionState {
    /// The `server_tick` of the anchor frame or sample that set it.
    pub tick: u64,
    /// Where the entity was.
    pub point: Point,
    /// Whether the entity had a route.
    pub moving: bool,
}

/// The newest sample's clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SampleClock {
    /// The newest sample's `server_tick`.
    pub server_tick: u64,
    /// The `processed_seq` that sample carried.
    pub processed_seq: u64,
}

/// The engine-neutral client replica of one filtered v4 session.
///
/// One per session; no serde or wire form, no restore constructor (a saved
/// replica would restore stale authorization, `tasks/T027c.md:22-23`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaWorldV4 {
    inner: ReplicaWorldV3,
    positions: BTreeMap<NetId, PositionState>,
    sample_clock: Option<SampleClock>,
}

impl ReplicaWorldV4 {
    /// A fresh, empty replica for `epoch`: no rows, no positions, no sample.
    pub fn new(epoch: SessionEpoch) -> Self {
        Self {
            inner: ReplicaWorldV3::new(epoch),
            positions: BTreeMap::new(),
            sample_clock: None,
        }
    }

    /// The wrapped v3 replica; every v3 query is answered here.
    pub fn v3(&self) -> &ReplicaWorldV3 {
        &self.inner
    }

    /// One entity's disclosed position; `None` when none is disclosed.
    pub fn position(&self, entity: NetId) -> Option<&PositionState> {
        self.positions.get(&entity)
    }

    /// Every disclosed position, ascending `NetId`. Every key is a live row
    /// of [`Self::v3`].
    pub fn positions(&self) -> &BTreeMap<NetId, PositionState> {
        &self.positions
    }

    /// The newest sample's clock; `None` before any sample.
    pub fn sample_clock(&self) -> Option<SampleClock> {
        self.sample_clock
    }

    /// Applies one decoded v4 lane-0 delta in strict sequence order.
    ///
    /// A refusal is fatal to the session and leaves the replica equal to its
    /// pre-call clone.
    pub fn apply_delta(&mut self, frame: &DeltaFrame) -> Result<(), ReplicaError> {
        if frame.lane != LANE_COMBAT {
            return Err(ReplicaError::WrongLane { got: frame.lane });
        }
        if frame.ops.len() > MAX_DELTA_OPS {
            return Err(ReplicaError::TooManyOps {
                got: frame.ops.len(),
            });
        }
        let expected = self.inner.next_event_seq();
        if frame.event_seq != expected || frame.event_seq > SEQ_LAST {
            return Err(ReplicaError::EventSeq {
                expected,
                got: frame.event_seq,
            });
        }

        // Validate and apply on a working copy of the whole replica; commit
        // only on success.
        let mut working = self.clone();
        for op in &frame.ops {
            working.apply_op(op, frame.server_tick)?;
        }
        // The sequence and tick advance through the inner replica, by an empty
        // v3 frame that cannot fail after the checks above.
        working.inner.apply_delta(&DeltaFrameV3 {
            lane: LANE_COMBAT,
            server_tick: frame.server_tick,
            event_seq: frame.event_seq,
            ops: Vec::new(),
        })?;
        *self = working;
        Ok(())
    }

    /// Replaces the replica with one published v3 snapshot body.
    ///
    /// Positions are emptied (the next page re-anchors them); the sample clock
    /// is kept. A refusal is fatal to the session and changes nothing.
    pub fn apply_snapshot(&mut self, publication: &PublicationV3) -> Result<(), ReplicaError> {
        self.inner.apply_snapshot(publication)?;
        self.positions.clear();
        Ok(())
    }

    /// Applies one position sample; never fails. Returns how many entries
    /// were stored.
    ///
    /// The clock moves only to a strictly newer `server_tick`. An entry is
    /// stored only for an entity that already has a disclosed position and
    /// only if the sample is strictly newer than that position's tick (newest
    /// wins, ADR-0036 L4); any other entry is skipped.
    pub fn apply_sample(&mut self, sample: &MoveSample) -> usize {
        let newer = match self.sample_clock {
            None => true,
            Some(clock) => sample.server_tick > clock.server_tick,
        };
        if newer {
            self.sample_clock = Some(SampleClock {
                server_tick: sample.server_tick,
                processed_seq: sample.processed_seq,
            });
        }
        let mut stored = 0;
        for entry in &sample.entries {
            if let Some(state) = self.positions.get_mut(&entry.entity) {
                if sample.server_tick > state.tick {
                    *state = PositionState {
                        tick: sample.server_tick,
                        point: entry.point,
                        moving: entry.moving,
                    };
                    stored += 1;
                }
            }
        }
        stored
    }

    /// Applies one op to the working state, in the contract's check order.
    fn apply_op(&mut self, op: &DeltaOp, server_tick: u64) -> Result<(), ReplicaError> {
        match op {
            DeltaOp::V3(op) => {
                self.inner.apply_op(op)?;
                if let DeltaOpV3::EntityLeave { entity } | DeltaOpV3::Despawned { entity } = op {
                    self.positions.remove(entity);
                }
                Ok(())
            }
            // An unchecked notice: the replica stores nothing from it.
            DeltaOp::Moved { .. } => Ok(()),
            DeltaOp::Position { entity, position } => {
                if self.inner.entity(*entity).is_none() {
                    return Err(ReplicaError::UnknownEntity { entity: *entity });
                }
                match position {
                    Some(fact) => {
                        let keep = self
                            .positions
                            .get(entity)
                            .is_some_and(|state| state.tick > server_tick);
                        if !keep {
                            self.positions.insert(
                                *entity,
                                PositionState {
                                    tick: server_tick,
                                    point: fact.point,
                                    moving: fact.moving,
                                },
                            );
                        }
                    }
                    None => {
                        self.positions.remove(entity);
                    }
                }
                Ok(())
            }
        }
    }
}
