//! Engine-neutral client replica of one filtered v2 session (T046, ADR-0035).
//!
//! [`ReplicaWorld`] applies decoded lane-0 [`DeltaFrame`]s and published
//! snapshot [`Publication`]s in strict per-session order, keeping only live
//! rows, one optional turn fact and a few counters. It reads no `World`,
//! clock, RNG, I/O or configuration, mints nothing, and refuses every frame
//! that breaks the T027b wire rules N1–N9; every refusal is fatal to the
//! bridge (ADR-0033 code 53) and leaves the replica unchanged.

use core::fmt;

use crate::protocol::{
    self, NetId, SessionEpoch, LANE_COMBAT, MAX_DELTA_OPS, MAX_VISIBLE_ENTITIES, SEQ_LAST,
};
use crate::protocol_transfer::{HealthFact, TurnFact};
use crate::protocol_v2::{DeltaFrame, DeltaOp};
use crate::reassembly::Publication;

/// Largest live row count, reached only mid-page (D8). Not a wire bound.
pub const MAX_REPLICA_ENTITIES: usize = 2 * MAX_VISIBLE_ENTITIES;

/// One live entity. Rows are strictly ascending by `net_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaEntity {
    /// The client-scoped replica id.
    pub net_id: NetId,
    /// The last disclosed health fact; `None` while unknown, never zero.
    pub health: Option<HealthFact>,
}

/// Why a frame or publication was refused. Every variant is fatal to the
/// session (ADR-0033 code 53).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplicaError {
    /// `frame.lane != LANE_COMBAT`.
    WrongLane {
        /// The lane the frame carried.
        got: u8,
    },
    /// `frame.ops.len() > MAX_DELTA_OPS`.
    TooManyOps {
        /// The op count the frame carried.
        got: usize,
    },
    /// `event_seq` is not the next expected one, or is above `SEQ_LAST`.
    EventSeq {
        /// The next accepted `event_seq`.
        expected: u64,
        /// The `event_seq` the frame carried.
        got: u64,
    },
    /// An op names an id that is not live at that point.
    UnknownEntity {
        /// The id the op named.
        entity: NetId,
    },
    /// An `EntityEnter` names a live id.
    DuplicateEntity {
        /// The live id the enter named.
        entity: NetId,
    },
    /// An id at or below the high-water mark that is not live (N3, D6).
    ReusedEntityId {
        /// The retired or never-issued id.
        entity: NetId,
    },
    /// More than `MAX_REPLICA_ENTITIES` rows, or a body over
    /// `MAX_VISIBLE_ENTITIES`.
    TooManyEntities,
    /// A `Receipt` carries another epoch.
    WrongEpoch,
    /// A body row is not strictly above the previous row (duplicates too).
    SnapshotOrder {
        /// The offending row id.
        entity: NetId,
    },
    /// `cursor` is below the last applied `event_seq` or above `SEQ_LAST`.
    SnapshotCursor {
        /// The lowest accepted cursor (the last applied `event_seq`).
        min: u64,
        /// The cursor the publication carried.
        got: u64,
    },
}

impl fmt::Display for ReplicaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongLane { got } => write!(f, "WrongLane at replica: got {got}"),
            Self::TooManyOps { got } => write!(f, "TooManyOps at replica: got {got}"),
            Self::EventSeq { expected, got } => {
                write!(f, "EventSeq at replica: expected {expected}, got {got}")
            }
            Self::UnknownEntity { entity } => {
                write!(f, "UnknownEntity at replica: {}", entity.get())
            }
            Self::DuplicateEntity { entity } => {
                write!(f, "DuplicateEntity at replica: {}", entity.get())
            }
            Self::ReusedEntityId { entity } => {
                write!(f, "ReusedEntityId at replica: {}", entity.get())
            }
            Self::TooManyEntities => f.write_str("TooManyEntities at replica"),
            Self::WrongEpoch => f.write_str("WrongEpoch at replica"),
            Self::SnapshotOrder { entity } => {
                write!(f, "SnapshotOrder at replica: {}", entity.get())
            }
            Self::SnapshotCursor { min, got } => {
                write!(f, "SnapshotCursor at replica: min {min}, got {got}")
            }
        }
    }
}

impl std::error::Error for ReplicaError {}

/// The engine-neutral client replica of one filtered v2 session. One per
/// session; no serde or wire form, no restore constructor (a saved replica
/// would restore stale authorization, `tasks/T027c.md:22-23`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaWorld {
    epoch: SessionEpoch,
    entities: Vec<ReplicaEntity>, // strictly ascending net_id
    turn: Option<TurnFact>,
    next_event_seq: u64, // 1 at `new`
    server_tick: u64,    // 0 at `new`
    high_water: u64,     // largest NetId ever stored; 0 at `new`
}

impl ReplicaWorld {
    /// A fresh, empty replica for `epoch`. The epoch never changes.
    pub fn new(epoch: SessionEpoch) -> Self {
        Self {
            epoch,
            entities: Vec::new(),
            turn: None,
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
    pub fn entities(&self) -> &[ReplicaEntity] {
        &self.entities
    }

    /// One live row by binary search; `None` for any id not live.
    pub fn entity(&self, net_id: NetId) -> Option<&ReplicaEntity> {
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

    /// The only `event_seq` `apply_delta` accepts next. `u64::MAX` after a
    /// publication at `SEQ_LAST`: nothing more is accepted.
    pub fn next_event_seq(&self) -> u64 {
        self.next_event_seq
    }

    /// The last accepted frame's `server_tick` or publication's `base_tick`.
    pub fn server_tick(&self) -> u64 {
        self.server_tick
    }

    /// Applies one decoded v2 lane-0 delta in strict sequence order.
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

    /// Replaces the replica with one published snapshot body (N5).
    ///
    /// A refusal is fatal to the session and leaves the replica equal to its
    /// pre-call clone.
    pub fn apply_snapshot(&mut self, publication: &Publication) -> Result<(), ReplicaError> {
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
        if let Some(active) = body.turn.as_ref().and_then(|turn| turn.active) {
            if !body.entities.iter().any(|row| row.entity == active) {
                return Err(ReplicaError::UnknownEntity { entity: active });
            }
        }

        // Validation passed, so commit the replacement in one step.
        self.entities = body
            .entities
            .iter()
            .map(|row| ReplicaEntity {
                net_id: row.entity,
                health: row.health.clone(),
            })
            .collect();
        self.turn = body.turn.clone();
        self.next_event_seq = publication.cursor + 1;
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
            DeltaOp::Legacy(op) => self.apply_legacy(op),
            DeltaOp::ActionResolved { actor, target, .. } => {
                self.require_live(*actor)?;
                self.require_live(*target)
            }
            DeltaOp::TurnStarted { actor, .. } => self.require_live(*actor),
            DeltaOp::EncounterEnded { .. } => Ok(()),
        }
    }

    /// Applies one unchanged v1 op.
    fn apply_legacy(&mut self, op: &protocol::DeltaOp) -> Result<(), ReplicaError> {
        match op {
            protocol::DeltaOp::EntityEnter { entity } => {
                if self.index_of(*entity).is_some() {
                    return Err(ReplicaError::DuplicateEntity { entity: *entity });
                }
                if entity.get() <= self.high_water {
                    return Err(ReplicaError::ReusedEntityId { entity: *entity });
                }
                if self.entities.len() >= MAX_REPLICA_ENTITIES {
                    return Err(ReplicaError::TooManyEntities);
                }
                self.entities.push(ReplicaEntity {
                    net_id: *entity,
                    health: None,
                });
                self.high_water = entity.get();
                Ok(())
            }
            protocol::DeltaOp::EntityLeave { entity } | protocol::DeltaOp::Despawned { entity } => {
                let index = self
                    .index_of(*entity)
                    .ok_or(ReplicaError::UnknownEntity { entity: *entity })?;
                self.entities.remove(index);
                Ok(())
            }
            protocol::DeltaOp::Spawned { entity } | protocol::DeltaOp::Died { entity } => {
                self.require_live(*entity)
            }
            protocol::DeltaOp::Health {
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
            protocol::DeltaOp::Turn { active, round } => {
                if let Some(active) = active {
                    self.require_live(*active)?;
                }
                self.turn = Some(TurnFact {
                    active: *active,
                    round: *round,
                });
                Ok(())
            }
            protocol::DeltaOp::Receipt { epoch, .. } => {
                if *epoch != self.epoch.0 {
                    return Err(ReplicaError::WrongEpoch);
                }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol_transfer::{SnapshotBody, SnapshotEntity, TransferId};
    use crate::protocol_v2::DeltaOp as V2Op;

    fn epoch(byte: u8) -> SessionEpoch {
        SessionEpoch([byte; 16])
    }

    fn id(raw: u64) -> NetId {
        NetId::new(raw).expect("nonzero id")
    }

    fn enter(entity: u64) -> DeltaOp {
        DeltaOp::Legacy(protocol::DeltaOp::EntityEnter { entity: id(entity) })
    }

    fn leave(entity: u64) -> DeltaOp {
        DeltaOp::Legacy(protocol::DeltaOp::EntityLeave { entity: id(entity) })
    }

    fn despawned(entity: u64) -> DeltaOp {
        DeltaOp::Legacy(protocol::DeltaOp::Despawned { entity: id(entity) })
    }

    fn health(entity: u64, health: u32, max_health: u32, dead: bool) -> DeltaOp {
        DeltaOp::Legacy(protocol::DeltaOp::Health {
            entity: id(entity),
            health,
            max_health,
            dead,
        })
    }

    fn turn(active: Option<u64>, round: u64) -> DeltaOp {
        DeltaOp::Legacy(protocol::DeltaOp::Turn {
            active: active.map(id),
            round,
        })
    }

    fn receipt(epoch: [u8; 16]) -> DeltaOp {
        DeltaOp::Legacy(protocol::DeltaOp::Receipt {
            epoch,
            lane: LANE_COMBAT,
            seq: 1,
            processed_tick: 0,
            status: protocol::ReceiptStatus::Applied,
        })
    }

    fn frame(seq: u64, tick: u64, ops: Vec<DeltaOp>) -> DeltaFrame {
        DeltaFrame {
            lane: LANE_COMBAT,
            server_tick: tick,
            event_seq: seq,
            ops,
        }
    }

    fn publication(
        cursor: u64,
        base_tick: u64,
        rows: Vec<(u64, Option<HealthFact>)>,
        turn: Option<TurnFact>,
    ) -> Publication {
        Publication {
            transfer: TransferId::new(1).expect("transfer id"),
            base_tick,
            cursor,
            body: SnapshotBody {
                entities: rows
                    .into_iter()
                    .map(|(entity, health)| SnapshotEntity {
                        entity: id(entity),
                        health,
                    })
                    .collect(),
                turn,
            },
        }
    }

    fn fact(health: u32, max_health: u32, dead: bool) -> HealthFact {
        HealthFact {
            health,
            max_health,
            dead,
        }
    }

    fn row(net_id: u64, health: Option<HealthFact>) -> ReplicaEntity {
        ReplicaEntity {
            net_id: id(net_id),
            health,
        }
    }

    // Behaviour item 1 — construction.
    #[test]
    fn construction_is_empty_and_starts_at_sequence_one() {
        let replica = ReplicaWorld::new(epoch(7));
        assert_eq!(replica.epoch(), epoch(7));
        assert!(replica.is_empty());
        assert_eq!(replica.len(), 0);
        assert_eq!(replica.entities(), &[]);
        assert_eq!(replica.entity(id(1)), None);
        assert_eq!(replica.turn(), None);
        assert_eq!(replica.next_event_seq(), 1);
        assert_eq!(replica.server_tick(), 0);
        assert_eq!(replica.high_water, 0);
    }

    // Behaviour items 2, 3 and 4 — precedence, atomicity, sequence law.
    #[test]
    fn delta_precedence_is_lane_then_ops_then_sequence_then_op_order() {
        let mut replica = ReplicaWorld::new(epoch(1));
        let mut ops = vec![leave(9)];
        ops.extend((1..=MAX_DELTA_OPS).map(|raw| enter(raw as u64)));
        let mut bad_lane = frame(9, 0, ops.clone());
        bad_lane.lane = 1;
        assert_eq!(
            replica.apply_delta(&bad_lane),
            Err(ReplicaError::WrongLane { got: 1 })
        );
        assert_eq!(
            replica.apply_delta(&frame(9, 0, ops)),
            Err(ReplicaError::TooManyOps {
                got: MAX_DELTA_OPS + 1
            })
        );
        assert_eq!(
            replica.apply_delta(&frame(3, 0, vec![leave(9)])),
            Err(ReplicaError::EventSeq {
                expected: 1,
                got: 3
            })
        );
        assert_eq!(
            replica.apply_delta(&frame(1, 0, vec![leave(9), enter(1)])),
            Err(ReplicaError::UnknownEntity { entity: id(9) })
        );
        assert_eq!(replica, ReplicaWorld::new(epoch(1)));
    }

    #[test]
    fn sequence_law_accepts_in_order_and_refuses_every_break() {
        let mut replica = ReplicaWorld::new(epoch(1));
        replica.apply_delta(&frame(1, 9, vec![enter(1)])).unwrap();
        replica.apply_delta(&frame(2, 5, vec![])).unwrap();
        replica.apply_delta(&frame(3, 4, vec![enter(2)])).unwrap();
        assert_eq!(replica.next_event_seq(), 4);
        assert_eq!(replica.server_tick(), 4);
        assert_eq!(replica.len(), 2);

        for (seq, expected) in [
            (
                1,
                ReplicaError::EventSeq {
                    expected: 4,
                    got: 1,
                },
            ),
            (
                0,
                ReplicaError::EventSeq {
                    expected: 4,
                    got: 0,
                },
            ),
            (
                5,
                ReplicaError::EventSeq {
                    expected: 4,
                    got: 5,
                },
            ),
            (
                SEQ_LAST + 1,
                ReplicaError::EventSeq {
                    expected: 4,
                    got: SEQ_LAST + 1,
                },
            ),
        ] {
            let before = replica.clone();
            assert_eq!(replica.apply_delta(&frame(seq, 0, vec![])), Err(expected));
            assert_eq!(replica, before);
        }
        replica.apply_delta(&frame(4, 0, vec![])).unwrap();
    }

    #[test]
    fn a_publication_at_seq_last_leaves_nothing_accepted() {
        let mut replica = ReplicaWorld::new(epoch(1));
        replica
            .apply_snapshot(&publication(SEQ_LAST, 3, vec![(1, None)], None))
            .unwrap();
        assert_eq!(replica.next_event_seq(), u64::MAX);
        assert_eq!(
            replica.apply_delta(&frame(u64::MAX, 0, vec![])),
            Err(ReplicaError::EventSeq {
                expected: u64::MAX,
                got: u64::MAX
            })
        );
    }

    // Behaviour items 5 and 12 — fresh replica and publication first.
    #[test]
    fn a_publication_may_precede_the_first_delta() {
        let mut replica = ReplicaWorld::new(epoch(1));
        replica
            .apply_snapshot(&publication(0, 8, vec![(1, None)], None))
            .unwrap();
        assert_eq!(replica.next_event_seq(), 1);
        replica.apply_delta(&frame(1, 9, vec![])).unwrap();
        assert_eq!(replica.next_event_seq(), 2);
    }

    // Behaviour item 6 — op effects and their check order.
    #[test]
    fn ops_have_their_contracted_effects() {
        let mut replica = ReplicaWorld::new(epoch(1));
        replica
            .apply_delta(&frame(
                1,
                10,
                vec![
                    enter(1),
                    health(1, 3, 10, false),
                    DeltaOp::Legacy(protocol::DeltaOp::Spawned { entity: id(1) }),
                    DeltaOp::Legacy(protocol::DeltaOp::Died { entity: id(1) }),
                    turn(Some(1), 2),
                    V2Op::ActionResolved {
                        actor: id(1),
                        target: id(1),
                        ability: crpg_core::Ulid::from_u128(1),
                        outcome: "success".to_string(),
                        damage: 0,
                    },
                    V2Op::TurnStarted {
                        actor: id(1),
                        round: 2,
                    },
                    V2Op::EncounterEnded {
                        encounter: crpg_core::Ulid::from_u128(2),
                        round: 2,
                    },
                    receipt([1; 16]),
                ],
            ))
            .unwrap();
        assert_eq!(replica.entities(), &[row(1, Some(fact(3, 10, false)))]);
        assert_eq!(
            replica.turn(),
            Some(&TurnFact {
                active: Some(id(1)),
                round: 2
            })
        );
        assert_eq!(replica.server_tick(), 10);

        assert_eq!(
            replica.apply_delta(&frame(
                2,
                10,
                vec![V2Op::ActionResolved {
                    actor: id(1),
                    target: id(9),
                    ability: crpg_core::Ulid::from_u128(1),
                    outcome: "success".to_string(),
                    damage: 0,
                }]
            )),
            Err(ReplicaError::UnknownEntity { entity: id(9) })
        );
        assert_eq!(
            replica.apply_delta(&frame(2, 10, vec![receipt([2; 16])])),
            Err(ReplicaError::WrongEpoch)
        );
        assert_eq!(
            replica.apply_delta(&frame(
                2,
                10,
                vec![V2Op::TurnStarted {
                    actor: id(9),
                    round: 2
                }]
            )),
            Err(ReplicaError::UnknownEntity { entity: id(9) })
        );
    }

    #[test]
    fn an_enter_is_legal_before_later_ops_in_the_same_frame_only() {
        let mut replica = ReplicaWorld::new(epoch(1));
        replica
            .apply_delta(&frame(1, 0, vec![enter(1), health(1, 1, 1, false)]))
            .unwrap();
        assert_eq!(replica.len(), 1);
        assert_eq!(
            replica.apply_delta(&frame(2, 0, vec![health(2, 1, 1, false), enter(2)])),
            Err(ReplicaError::UnknownEntity { entity: id(2) })
        );
        assert_eq!(replica.len(), 1);
    }

    // Behaviour item 7 — health.
    #[test]
    fn unknown_health_is_not_zero_and_health_replaces_every_field() {
        let mut replica = ReplicaWorld::new(epoch(1));
        replica.apply_delta(&frame(1, 0, vec![enter(1)])).unwrap();
        assert_eq!(replica.entity(id(1)), Some(&row(1, None)));
        replica
            .apply_delta(&frame(2, 0, vec![health(1, 0, 10, false)]))
            .unwrap();
        assert_eq!(
            replica.entity(id(1)),
            Some(&row(1, Some(fact(0, 10, false))))
        );
        replica
            .apply_delta(&frame(3, 0, vec![health(1, 4, 7, true)]))
            .unwrap();
        assert_eq!(replica.entity(id(1)), Some(&row(1, Some(fact(4, 7, true)))));
        assert_eq!(replica.len(), 1);
        assert_eq!(replica.entity(id(2)), None);
        assert_ne!(row(1, None), row(2, None));
    }

    #[test]
    fn died_never_changes_health() {
        let mut replica = ReplicaWorld::new(epoch(1));
        replica
            .apply_delta(&frame(1, 0, vec![enter(1), enter(2)]))
            .unwrap();
        let died = |entity| DeltaOp::Legacy(protocol::DeltaOp::Died { entity: id(entity) });
        replica
            .apply_delta(&frame(2, 0, vec![died(1), died(2)]))
            .unwrap();
        assert_eq!(replica.entity(id(1)), Some(&row(1, None)));
        assert_eq!(replica.entity(id(2)), Some(&row(2, None)));
        replica
            .apply_delta(&frame(3, 0, vec![health(2, 0, 1, false)]))
            .unwrap();
        replica.apply_delta(&frame(4, 0, vec![died(2)])).unwrap();
        assert_eq!(
            replica.entity(id(2)),
            Some(&row(2, Some(fact(0, 1, false))))
        );
    }

    // Behaviour items 8 and 9 — turn and removal.
    #[test]
    fn turn_is_replaced_verbatim_and_is_never_cleared_by_a_delta() {
        let mut replica = ReplicaWorld::new(epoch(1));
        replica
            .apply_delta(&frame(
                1,
                0,
                vec![enter(1), turn(Some(1), 1), turn(None, 1)],
            ))
            .unwrap();
        assert_eq!(
            replica.turn(),
            Some(&TurnFact {
                active: None,
                round: 1
            })
        );
        replica
            .apply_delta(&frame(2, 0, vec![turn(Some(1), 2), leave(1)]))
            .unwrap();
        assert_eq!(
            replica.turn(),
            Some(&TurnFact {
                active: Some(id(1)),
                round: 2
            })
        );
        assert_eq!(replica.turn().and_then(|turn| turn.active), Some(id(1)));
        replica
            .apply_delta(&frame(
                3,
                0,
                vec![V2Op::EncounterEnded {
                    encounter: crpg_core::Ulid::from_u128(3),
                    round: 2,
                }],
            ))
            .unwrap();
        assert_eq!(replica.turn().and_then(|turn| turn.active), Some(id(1)));
    }

    #[test]
    fn removal_is_exactly_one_op_and_retires_the_id() {
        let mut replica = ReplicaWorld::new(epoch(1));
        replica
            .apply_delta(&frame(
                1,
                0,
                vec![enter(1), enter(2), health(2, 2, 2, false)],
            ))
            .unwrap();
        replica
            .apply_delta(&frame(2, 0, vec![despawned(2)]))
            .unwrap();
        assert_eq!(replica.entities(), &[row(1, None)]);
        for op in [
            health(2, 1, 1, false),
            leave(2),
            despawned(2),
            DeltaOp::Legacy(protocol::DeltaOp::Spawned { entity: id(2) }),
        ] {
            assert_eq!(
                replica.apply_delta(&frame(3, 0, vec![op])),
                Err(ReplicaError::UnknownEntity { entity: id(2) })
            );
        }
        assert_eq!(
            replica.apply_delta(&frame(3, 0, vec![enter(2)])),
            Err(ReplicaError::ReusedEntityId { entity: id(2) })
        );
    }

    // Behaviour item 10 — bounds.
    #[test]
    fn the_mid_page_bound_is_two_thousand_and_forty_eight() {
        let mut replica = ReplicaWorld::new(epoch(1));
        let mut seq = 1;
        for chunk in 0..8u64 {
            let ops = (0..MAX_DELTA_OPS)
                .map(|index| enter(chunk * MAX_DELTA_OPS as u64 + index as u64 + 1))
                .collect();
            replica.apply_delta(&frame(seq, 0, ops)).unwrap();
            seq += 1;
        }
        assert_eq!(replica.len(), MAX_REPLICA_ENTITIES);
        assert_eq!(
            replica.apply_delta(&frame(seq, 0, vec![enter(MAX_REPLICA_ENTITIES as u64 + 1)])),
            Err(ReplicaError::TooManyEntities)
        );
        replica
            .apply_delta(&frame(
                seq,
                0,
                vec![leave(1), enter(MAX_REPLICA_ENTITIES as u64 + 1)],
            ))
            .unwrap();
        assert_eq!(replica.len(), MAX_REPLICA_ENTITIES);
    }

    // Behaviour items 11 and 12 — snapshot precedence and replacement.
    #[test]
    fn snapshot_checks_in_contract_order() {
        let mut replica = ReplicaWorld::new(epoch(1));
        replica
            .apply_delta(&frame(1, 0, vec![enter(1), enter(2), turn(Some(1), 1)]))
            .unwrap();
        replica
            .apply_delta(&frame(2, 0, vec![enter(4), leave(4)]))
            .unwrap();
        let before = replica.clone();
        assert_eq!(
            replica.apply_snapshot(&publication(1, 0, vec![], None)),
            Err(ReplicaError::SnapshotCursor { min: 2, got: 1 })
        );
        assert_eq!(
            replica.apply_snapshot(&publication(SEQ_LAST + 1, 0, vec![], None)),
            Err(ReplicaError::SnapshotCursor {
                min: 2,
                got: SEQ_LAST + 1
            })
        );
        let too_many: Vec<_> = (1..=MAX_VISIBLE_ENTITIES as u64 + 1)
            .map(|entity| (entity, None))
            .collect();
        assert_eq!(
            replica.apply_snapshot(&publication(2, 0, too_many, None)),
            Err(ReplicaError::TooManyEntities)
        );
        assert_eq!(
            replica.apply_snapshot(&publication(2, 0, vec![(4, None)], None)),
            Err(ReplicaError::ReusedEntityId { entity: id(4) })
        );
        assert_eq!(
            replica.apply_snapshot(&publication(
                2,
                0,
                vec![(1, None)],
                Some(TurnFact {
                    active: Some(id(9)),
                    round: 1,
                }),
            )),
            Err(ReplicaError::UnknownEntity { entity: id(9) })
        );
        assert_eq!(replica, before);

        // Order is checked on rows whose ids are otherwise admissible.
        let mut ordered = ReplicaWorld::new(epoch(1));
        ordered
            .apply_delta(&frame(1, 0, vec![enter(1), enter(2)]))
            .unwrap();
        let before = ordered.clone();
        for rows in [vec![(3, None), (2, None)], vec![(2, None), (2, None)]] {
            assert_eq!(
                ordered.apply_snapshot(&publication(1, 0, rows, None)),
                Err(ReplicaError::SnapshotOrder { entity: id(2) })
            );
            assert_eq!(ordered, before);
        }
    }

    #[test]
    fn snapshot_replaces_rows_and_turn_wholesale() {
        let mut replica = ReplicaWorld::new(epoch(1));
        replica
            .apply_delta(&frame(
                1,
                0,
                vec![
                    enter(1),
                    enter(2),
                    enter(3),
                    health(1, 1, 1, false),
                    turn(Some(3), 2),
                ],
            ))
            .unwrap();
        replica
            .apply_snapshot(&publication(
                5,
                40,
                vec![(1, None), (3, Some(fact(2, 3, false))), (4, None)],
                None,
            ))
            .unwrap();
        assert_eq!(
            replica.entities(),
            &[row(1, None), row(3, Some(fact(2, 3, false))), row(4, None)]
        );
        assert_eq!(replica.turn(), None);
        assert_eq!(replica.next_event_seq(), 6);
        assert_eq!(replica.server_tick(), 40);
        assert_eq!(replica.entity(id(2)), None);
        assert_eq!(
            replica.apply_delta(&frame(6, 41, vec![health(2, 1, 1, false)])),
            Err(ReplicaError::UnknownEntity { entity: id(2) })
        );
        assert_eq!(
            replica.apply_delta(&frame(6, 41, vec![enter(2)])),
            Err(ReplicaError::ReusedEntityId { entity: id(2) })
        );
        replica.apply_delta(&frame(6, 41, vec![enter(5)])).unwrap();

        replica
            .apply_snapshot(&publication(6, 42, vec![], None))
            .unwrap();
        assert!(replica.is_empty());
        assert_eq!(replica.turn(), None);
        assert_eq!(replica.high_water, 5);
        assert_eq!(
            replica.apply_delta(&frame(7, 43, vec![enter(3)])),
            Err(ReplicaError::ReusedEntityId { entity: id(3) })
        );
        replica.apply_delta(&frame(7, 43, vec![enter(6)])).unwrap();
    }

    // Behaviour items 13 and 14 — queries and determinism.
    #[test]
    fn queries_agree_and_equal_inputs_give_equal_replicas() {
        let mut left = ReplicaWorld::new(epoch(1));
        let mut right = ReplicaWorld::new(epoch(1));
        let ops = vec![enter(1), enter(2), enter(4), health(1, 5, 6, false)];
        left.apply_delta(&frame(1, 7, ops.clone())).unwrap();
        right.apply_delta(&frame(1, 7, ops)).unwrap();
        assert_eq!(left, right);
        assert_eq!(
            left.entities()
                .iter()
                .map(|row| row.net_id.get())
                .collect::<Vec<_>>(),
            vec![1, 2, 4]
        );
        for entity in 1..=4 {
            assert_eq!(
                left.entity(id(entity)).is_some(),
                left.entities().iter().any(|row| row.net_id == id(entity))
            );
        }
        assert_eq!(left.len(), left.entities().len());
        assert!(!left.is_empty());
        assert_eq!(left.entity(id(3)), None);
        assert_eq!(left.entity(id(99)), None);

        right
            .apply_delta(&frame(2, 7, vec![health(2, 1, 1, true)]))
            .unwrap();
        left.apply_delta(&frame(2, 7, vec![health(2, 1, 1, false)]))
            .unwrap();
        assert_ne!(left, right);
        assert_eq!(left.next_event_seq(), right.next_event_seq());
        assert_eq!(left.server_tick(), right.server_tick());
    }

    // Behaviour item 3 — atomicity of a mid-frame failure.
    #[test]
    fn a_mid_frame_failure_changes_nothing() {
        let mut replica = ReplicaWorld::new(epoch(1));
        replica
            .apply_delta(&frame(
                1,
                0,
                vec![enter(1), enter(2), health(2, 1, 1, false)],
            ))
            .unwrap();
        let before = replica.clone();
        let ops = vec![
            enter(9),
            leave(1),
            health(2, 2, 2, true),
            turn(Some(9), 3),
            enter(9),
        ];
        assert_eq!(
            replica.apply_delta(&frame(2, 5, ops)),
            Err(ReplicaError::DuplicateEntity { entity: id(9) })
        );
        assert_eq!(replica, before);

        let ops = vec![enter(9), leave(1), health(2, 2, 2, true), turn(Some(9), 3)];
        replica.apply_delta(&frame(2, 5, ops)).unwrap();
        assert_eq!(
            replica.entities(),
            &[row(2, Some(fact(2, 2, true))), row(9, None)]
        );
        assert_eq!(
            replica.turn(),
            Some(&TurnFact {
                active: Some(id(9)),
                round: 3
            })
        );
        assert_eq!(replica.server_tick(), 5);
        assert_eq!(replica.next_event_seq(), 3);
    }

    // Behaviour item 15 — Display strings.
    #[test]
    fn display_strings_are_exact() {
        let cases = [
            (
                ReplicaError::WrongLane { got: 1 },
                "WrongLane at replica: got 1",
            ),
            (
                ReplicaError::TooManyOps { got: 257 },
                "TooManyOps at replica: got 257",
            ),
            (
                ReplicaError::EventSeq {
                    expected: 4,
                    got: 5,
                },
                "EventSeq at replica: expected 4, got 5",
            ),
            (
                ReplicaError::UnknownEntity { entity: id(7) },
                "UnknownEntity at replica: 7",
            ),
            (
                ReplicaError::DuplicateEntity { entity: id(7) },
                "DuplicateEntity at replica: 7",
            ),
            (
                ReplicaError::ReusedEntityId { entity: id(7) },
                "ReusedEntityId at replica: 7",
            ),
            (ReplicaError::TooManyEntities, "TooManyEntities at replica"),
            (ReplicaError::WrongEpoch, "WrongEpoch at replica"),
            (
                ReplicaError::SnapshotOrder { entity: id(2) },
                "SnapshotOrder at replica: 2",
            ),
            (
                ReplicaError::SnapshotCursor { min: 3, got: 2 },
                "SnapshotCursor at replica: min 3, got 2",
            ),
        ];
        for (error, expected) in cases {
            assert_eq!(error.to_string(), expected);
        }
        fn assert_copy_and_error<E: Copy + std::error::Error>(_error: E) {}
        assert_copy_and_error(ReplicaError::WrongEpoch);
    }
}
