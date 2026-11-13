//! Engine-neutral client replica of one filtered v3 session (T140r2,
//! ADR-0035).
//!
//! [`ReplicaWorldV3`] applies decoded v3 lane-0 [`DeltaFrame`]s and published
//! v3 snapshot [`PublicationV3`]s in strict per-session order. It holds what
//! T046's [`ReplicaWorld`](crate::replica::ReplicaWorld) holds — live rows,
//! the optional turn fact and the sequence counters — and what v3 adds: each
//! entity's fact set, the encounter image, the timeline image, the pause state
//! and the viewer's host facts.
//!
//! It reads no `World`, clock, RNG, I/O or configuration, mints nothing and
//! derives no presence, fact, ownership or legality. Every refusal is an
//! existing [`ReplicaError`] (no v3-specific variant): an op or body naming an
//! id that is not live, or not in the body, is `UnknownEntity`. A refusal is
//! fatal to the bridge (ADR-0033 code 53) and leaves the replica unchanged.

use crate::pool_v3::PublicationV3;
use crate::protocol::{LANE_COMBAT, MAX_DELTA_OPS, MAX_VISIBLE_ENTITIES, SEQ_LAST};
use crate::protocol_transfer::{HealthFact, TurnFact};
use crate::protocol_v3::{
    DeltaFrame, DeltaOp, EncounterFacts, FactSet, HostFacts, NetId, PauseReason, SessionEpoch,
    TimelineEntry,
};

pub use crate::replica::{ReplicaError, MAX_REPLICA_ENTITIES};

/// One live v3 row. Rows are strictly ascending by `net_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaEntityV3 {
    /// The client-scoped replica id.
    pub net_id: NetId,
    /// The last disclosed health fact; `None` while unknown, never zero.
    pub health: Option<HealthFact>,
    /// The viewer's complete fact image; [`FactSet::default()`] is every
    /// family withheld.
    pub facts: FactSet,
}

/// The last `Clock` op's pause state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseState {
    /// `Clock { paused: None }`: the session is running.
    Running,
    /// `Clock { paused: Some(reason) }`; `reason` `None` is withheld.
    Paused {
        /// Why the host is paused, when disclosed.
        reason: Option<PauseReason>,
    },
}

/// The engine-neutral client replica of one filtered v3 session.
///
/// One per session; no serde or wire form, no restore constructor (a saved
/// replica would restore stale authorization, `tasks/T027c.md:22-23`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaWorldV3 {
    epoch: SessionEpoch,
    entities: Vec<ReplicaEntityV3>, // strictly ascending net_id
    turn: Option<TurnFact>,
    encounter: EncounterFacts,
    timeline: Option<Vec<TimelineEntry>>,
    pause: Option<PauseState>,
    host_facts: Option<HostFacts>,
    next_event_seq: u64, // 1 at `new`
    server_tick: u64,    // 0 at `new`
    high_water: u64,     // largest NetId ever stored; 0 at `new`
}

impl ReplicaWorldV3 {
    /// A fresh, empty replica for `epoch`. The epoch never changes.
    pub fn new(epoch: SessionEpoch) -> Self {
        Self {
            epoch,
            entities: Vec::new(),
            turn: None,
            encounter: EncounterFacts::default(),
            timeline: None,
            pause: None,
            host_facts: None,
            next_event_seq: 1,
            server_tick: 0,
            high_water: 0,
        }
    }

    /// The session generation this replica belongs to.
    pub fn epoch(&self) -> SessionEpoch {
        self.epoch
    }

    /// Every live row, ascending `NetId`; at most `MAX_REPLICA_ENTITIES`.
    pub fn entities(&self) -> &[ReplicaEntityV3] {
        &self.entities
    }

    /// One live row by binary search; `None` for any id not live.
    pub fn entity(&self, net_id: NetId) -> Option<&ReplicaEntityV3> {
        self.index_of(net_id).map(|index| &self.entities[index])
    }

    /// The number of live rows.
    pub fn len(&self) -> usize {
        self.entities.len()
    }

    /// Whether no row is live.
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    /// The last applied turn fact; `None` before any in this replica.
    pub fn turn(&self) -> Option<&TurnFact> {
        self.turn.as_ref()
    }

    /// The last `Encounter` image; default is every encounter family withheld.
    pub fn encounter(&self) -> &EncounterFacts {
        &self.encounter
    }

    /// The last `Timeline` image; `None` is withheld (forget the timeline).
    pub fn timeline(&self) -> Option<&[TimelineEntry]> {
        self.timeline.as_deref()
    }

    /// The last `Clock` op's pause state; `None` before the first one.
    pub fn pause(&self) -> Option<PauseState> {
        self.pause
    }

    /// The last `HostFacts` op's value; `None` before the first one and kept
    /// across a snapshot.
    pub fn host_facts(&self) -> Option<&HostFacts> {
        self.host_facts.as_ref()
    }

    /// The only `event_seq` `apply_delta` accepts next. `u64::MAX` after a
    /// publication at `SEQ_LAST`: nothing more is accepted.
    pub fn next_event_seq(&self) -> u64 {
        self.next_event_seq
    }

    /// The last accepted frame's `server_tick` or publication's `base_tick`.
    pub fn server_tick(&self) -> u64 {
        self.server_tick
    }

    /// Applies one decoded v3 lane-0 delta in strict sequence order.
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
        let expected = self.next_event_seq;
        if frame.event_seq != expected || frame.event_seq > SEQ_LAST {
            return Err(ReplicaError::EventSeq {
                expected,
                got: frame.event_seq,
            });
        }

        // D9: validate and apply on a working copy, commit only on success.
        let mut working = self.clone();
        for op in &frame.ops {
            working.apply_op(op)?;
        }
        working.next_event_seq = frame.event_seq + 1;
        working.server_tick = frame.server_tick;
        *self = working;
        Ok(())
    }

    /// Replaces the replica with one published v3 snapshot body.
    ///
    /// The pause state and the host facts are host state, are not in the body,
    /// and are kept as they are. A refusal is fatal to the session and leaves
    /// the replica equal to its pre-call clone.
    pub fn apply_snapshot(&mut self, publication: &PublicationV3) -> Result<(), ReplicaError> {
        let last = self.next_event_seq.saturating_sub(1);
        if publication.cursor < last || publication.cursor > SEQ_LAST {
            return Err(ReplicaError::SnapshotCursor {
                min: last,
                got: publication.cursor,
            });
        }
        let body = &publication.body;
        if body.entities.len() > MAX_VISIBLE_ENTITIES {
            return Err(ReplicaError::TooManyEntities);
        }

        let mut previous: Option<NetId> = None;
        for row in &body.entities {
            if let Some(previous) = previous {
                if row.entity <= previous {
                    return Err(ReplicaError::SnapshotOrder { entity: row.entity });
                }
            }
            if row.entity.get() <= self.high_water && self.index_of(row.entity).is_none() {
                return Err(ReplicaError::ReusedEntityId { entity: row.entity });
            }
            previous = Some(row.entity);
        }
        for row in &body.entities {
            if let Some(Some(order)) = &row.facts.order {
                if !body_contains(body, order.target) {
                    return Err(ReplicaError::UnknownEntity {
                        entity: order.target,
                    });
                }
            }
        }
        if let Some(active) = body.turn.as_ref().and_then(|turn| turn.active) {
            if !body_contains(body, active) {
                return Err(ReplicaError::UnknownEntity { entity: active });
            }
        }
        if let Some(entries) = &body.timeline {
            for entry in entries {
                if !body_contains(body, entry.entity) {
                    return Err(ReplicaError::UnknownEntity {
                        entity: entry.entity,
                    });
                }
            }
        }

        // Validation passed, so commit the replacement in one step.
        self.entities = body
            .entities
            .iter()
            .map(|row| ReplicaEntityV3 {
                net_id: row.entity,
                health: row.health.clone(),
                facts: row.facts.clone(),
            })
            .collect();
        self.turn = body.turn.clone();
        self.encounter = body.encounter;
        self.timeline = body.timeline.clone();
        self.next_event_seq = publication.cursor.saturating_add(1);
        self.server_tick = publication.base_tick;
        if let Some(last) = body.entities.last() {
            if last.entity.get() > self.high_water {
                self.high_water = last.entity.get();
            }
        }
        Ok(())
    }

    /// Applies one op to the working state, in the contract's check order.
    fn apply_op(&mut self, op: &DeltaOp) -> Result<(), ReplicaError> {
        match op {
            DeltaOp::EntityEnter { entity } => {
                if self.index_of(*entity).is_some() {
                    return Err(ReplicaError::DuplicateEntity { entity: *entity });
                }
                if entity.get() <= self.high_water {
                    return Err(ReplicaError::ReusedEntityId { entity: *entity });
                }
                if self.entities.len() >= MAX_REPLICA_ENTITIES {
                    return Err(ReplicaError::TooManyEntities);
                }
                self.entities.push(ReplicaEntityV3 {
                    net_id: *entity,
                    health: None,
                    facts: FactSet::default(),
                });
                self.high_water = entity.get();
                Ok(())
            }
            DeltaOp::EntityLeave { entity } | DeltaOp::Despawned { entity } => {
                let index = self
                    .index_of(*entity)
                    .ok_or(ReplicaError::UnknownEntity { entity: *entity })?;
                self.entities.remove(index);
                Ok(())
            }
            DeltaOp::Spawned { entity } | DeltaOp::Died { entity } => self.require_live(*entity),
            DeltaOp::Health {
                entity,
                health,
                max_health,
                dead,
            } => {
                let index = self
                    .index_of(*entity)
                    .ok_or(ReplicaError::UnknownEntity { entity: *entity })?;
                self.entities[index].health = Some(HealthFact {
                    health: *health,
                    max_health: *max_health,
                    dead: *dead,
                });
                Ok(())
            }
            DeltaOp::Turn { active, round } => {
                if let Some(active) = active {
                    self.require_live(*active)?;
                }
                self.turn = Some(TurnFact {
                    active: *active,
                    round: *round,
                });
                Ok(())
            }
            DeltaOp::Receipt { epoch, .. } => {
                if *epoch != self.epoch.0 {
                    return Err(ReplicaError::WrongEpoch);
                }
                Ok(())
            }
            DeltaOp::ActionResolved { actor, target, .. } => {
                self.require_live(*actor)?;
                self.require_live(*target)
            }
            DeltaOp::TurnStarted { actor, .. } => self.require_live(*actor),
            DeltaOp::EncounterEnded { .. } => Ok(()),
            DeltaOp::Encounter(facts) => {
                self.encounter = *facts;
                Ok(())
            }
            DeltaOp::Timeline { entries } => {
                if let Some(entries) = entries {
                    for entry in entries {
                        self.require_live(entry.entity)?;
                    }
                }
                self.timeline = entries.clone();
                Ok(())
            }
            DeltaOp::EntityFacts { entity, facts } => {
                let index = self
                    .index_of(*entity)
                    .ok_or(ReplicaError::UnknownEntity { entity: *entity })?;
                if let Some(Some(order)) = &facts.order {
                    self.require_live(order.target)?;
                }
                self.entities[index].facts = facts.clone();
                Ok(())
            }
            DeltaOp::RoundStarted { .. }
            | DeltaOp::ModeChanged { .. }
            | DeltaOp::OrderEnded { .. }
            | DeltaOp::Joined { .. }
            | DeltaOp::Left { .. } => Ok(()),
            DeltaOp::Clock { paused } => {
                self.pause = Some(match *paused {
                    None => PauseState::Running,
                    Some(reason) => PauseState::Paused { reason },
                });
                Ok(())
            }
            DeltaOp::HostFacts(facts) => {
                self.host_facts = Some(*facts);
                Ok(())
            }
        }
    }

    /// Accepts only an id present in the working rows (N1).
    fn require_live(&self, entity: NetId) -> Result<(), ReplicaError> {
        if self.index_of(entity).is_some() {
            Ok(())
        } else {
            Err(ReplicaError::UnknownEntity { entity })
        }
    }

    /// Binary search over the strictly ascending rows (D2).
    fn index_of(&self, net_id: NetId) -> Option<usize> {
        self.entities
            .binary_search_by(|row| row.net_id.cmp(&net_id))
            .ok()
    }
}

/// Whether `net` is a row of `body` (strictly ascending after the order
/// check, so a binary search is exact).
fn body_contains(body: &crate::transfer_v3::SnapshotBodyV3, net: NetId) -> bool {
    body.entities
        .binary_search_by_key(&net.get(), |row| row.entity.get())
        .is_ok()
}
