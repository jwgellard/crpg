//! T026p5 implementer suite: the protocol v4 client replica (`replica_v4`) and
//! the lane-1 sender state (`outbox_v4`), derived from `tasks/T026p.md`
//! Behaviour R-1..R-9 and O-1..O-4 and the tests R1..R9 and O1..O4, through the
//! public API only.
//!
//! Expectations are hand-written whole values: whole results, full error
//! values and whole replica state through the public accessors. Every refusal
//! is checked to leave the replica (or outbox) equal to its pre-call clone, and
//! each one has a passing control. R2 runs `ReplicaWorldV3` side by side as the
//! differential oracle. R9 drives a `ReassemblyPoolV4` end to end.

use std::collections::BTreeMap;
use std::error::Error;

use crpg_core::Ulid;
use crpg_net::codec_v4::encode_delta;
use crpg_net::movement_v4::{
    MoveBody, MoveIntent, MoveOutcome, MoveReceipt, MoveSample, SampleEntry, POLICY_MOVE_IN_FLIGHT,
};
use crpg_net::outbox_v4::{MoveOutbox, OutboxError};
use crpg_net::pool_v4::{
    DeltaOutcome, GateMode, PublicationV3, ReassemblyPoolV4, ReplicaUpdateV4, TransferLimitsV3,
    TransferOutcome,
};
use crpg_net::protocol::{MAX_DELTA_OPS, SEQ_LAST};
use crpg_net::protocol_transfer::TransferId;
use crpg_net::protocol_v3::{
    DeltaFrame as V3Frame, DeltaOp as V3Op, EncounterFacts, FactSet, ReceiptStatus as V3Status,
};
use crpg_net::protocol_v4::{
    DeltaFrame, DeltaOp, NetId, Point, PositionFact, RejectionCode, SessionEpoch,
};
use crpg_net::replica::ReplicaError;
use crpg_net::replica_v3::{ReplicaEntityV3, ReplicaWorldV3};
use crpg_net::replica_v4::{PositionState, ReplicaWorldV4, SampleClock};
use crpg_net::transfer_v3::{plan_transfer, SnapshotBodyV3, SnapshotEntityV3};

// ---------------------------------------------------------------------------
// Fixtures.
// ---------------------------------------------------------------------------

const Z16: [u8; 16] = [0; 16];

fn id(n: u64) -> NetId {
    NetId::new(n).expect("fixture net id is nonzero")
}

fn tid(n: u64) -> TransferId {
    TransferId::new(n).expect("fixture transfer id is nonzero")
}

fn u(n: u128) -> Ulid {
    Ulid::from_u128(n)
}

fn p(x: i32, y: i32, z: i32) -> Point {
    Point::new([x, y, z]).expect("fixture point is within U3")
}

fn a() -> Point {
    p(0, 0, 0)
}

fn b() -> Point {
    p(65_536, 0, -131_072)
}

fn replica() -> ReplicaWorldV4 {
    ReplicaWorldV4::new(SessionEpoch(Z16))
}

fn frame(seq: u64, tick: u64, ops: Vec<DeltaOp>) -> DeltaFrame {
    DeltaFrame {
        lane: 0,
        server_tick: tick,
        event_seq: seq,
        ops,
    }
}

/// The contract's `F(n, tick, ops)`.
fn f(seq: u64, tick: u64, ops: Vec<DeltaOp>) -> DeltaFrame {
    frame(seq, tick, ops)
}

fn v3(op: V3Op) -> DeltaOp {
    DeltaOp::V3(op)
}

fn enter(n: u64) -> DeltaOp {
    v3(V3Op::EntityEnter { entity: id(n) })
}

fn leave(n: u64) -> DeltaOp {
    v3(V3Op::EntityLeave { entity: id(n) })
}

fn despawned(n: u64) -> DeltaOp {
    v3(V3Op::Despawned { entity: id(n) })
}

fn pos(entity: u64, point: Point, moving: bool) -> DeltaOp {
    DeltaOp::Position {
        entity: id(entity),
        position: Some(PositionFact { point, moving }),
    }
}

fn forget(entity: u64) -> DeltaOp {
    DeltaOp::Position {
        entity: id(entity),
        position: None,
    }
}

fn moved(entity: u64, points: Vec<Point>) -> DeltaOp {
    DeltaOp::Moved {
        entity: id(entity),
        points,
    }
}

fn state(tick: u64, point: Point, moving: bool) -> PositionState {
    PositionState {
        tick,
        point,
        moving,
    }
}

fn entry(entity: u64, point: Point, moving: bool) -> SampleEntry {
    SampleEntry {
        entity: id(entity),
        point,
        moving,
    }
}

fn sample(tick: u64, processed_seq: u64, entries: Vec<SampleEntry>) -> MoveSample {
    MoveSample {
        server_tick: tick,
        processed_seq,
        entries,
    }
}

fn clock(server_tick: u64, processed_seq: u64) -> Option<SampleClock> {
    Some(SampleClock {
        server_tick,
        processed_seq,
    })
}

fn e0() -> SnapshotBodyV3 {
    SnapshotBodyV3 {
        entities: vec![],
        turn: None,
        encounter: EncounterFacts::default(),
        timeline: None,
    }
}

fn rows(ids: &[u64]) -> SnapshotBodyV3 {
    SnapshotBodyV3 {
        entities: ids
            .iter()
            .map(|n| SnapshotEntityV3 {
                entity: id(*n),
                health: None,
                facts: FactSet::default(),
            })
            .collect(),
        ..e0()
    }
}

fn snap(cursor: u64, base: u64, body: SnapshotBodyV3) -> PublicationV3 {
    PublicationV3 {
        transfer: tid(1),
        base_tick: base,
        cursor,
        body,
    }
}

/// The contract's "anchors": two rows and a position for each.
fn anchors() -> DeltaFrame {
    f(
        1,
        100,
        vec![enter(1), enter(2), pos(1, a(), false), pos(2, b(), true)],
    )
}

fn anchored() -> ReplicaWorldV4 {
    let mut replica = replica();
    assert_eq!(replica.apply_delta(&anchors()), Ok(()));
    replica
}

/// A refused frame returns exactly `expected` and changes nothing.
fn refused(replica: &mut ReplicaWorldV4, frame: &DeltaFrame, expected: ReplicaError) {
    let before = replica.clone();
    assert_eq!(replica.apply_delta(frame), Err(expected));
    assert_eq!(*replica, before, "a refused frame changed the replica");
}

/// The same frame as a v3 frame, for the v3 oracle: every op must be `V3`.
fn unwrap_v3(frame: &DeltaFrame) -> V3Frame {
    V3Frame {
        lane: frame.lane,
        server_tick: frame.server_tick,
        event_seq: frame.event_seq,
        ops: frame
            .ops
            .iter()
            .map(|op| match op {
                DeltaOp::V3(op) => op.clone(),
                other => panic!("not a v3 op: {other:?}"),
            })
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// R1 — construction.
// ---------------------------------------------------------------------------

#[test]
fn r1_construction() {
    let replica = replica();
    assert_eq!(*replica.v3(), ReplicaWorldV3::new(SessionEpoch(Z16)));
    assert_eq!(replica.v3().epoch(), SessionEpoch(Z16));
    assert_eq!(replica.v3().next_event_seq(), 1);
    assert_eq!(replica.v3().server_tick(), 0);
    assert!(replica.v3().is_empty());
    assert_eq!(replica.positions(), &BTreeMap::new());
    assert!(replica.positions().is_empty());
    assert_eq!(replica.position(id(1)), None);
    assert_eq!(replica.sample_clock(), None);

    // A non-zero epoch is the inner replica's epoch.
    let other = ReplicaWorldV4::new(SessionEpoch([7; 16]));
    assert_eq!(*other.v3(), ReplicaWorldV3::new(SessionEpoch([7; 16])));
    assert_ne!(other, replica);
}

// ---------------------------------------------------------------------------
// R2 — matches the v3 replica.
// ---------------------------------------------------------------------------

/// Runs one frame on both replicas, asserts the same `Result` and the same
/// state, and returns the result.
fn step(
    v3r: &mut ReplicaWorldV3,
    v4r: &mut ReplicaWorldV4,
    frame: DeltaFrame,
) -> Result<(), ReplicaError> {
    let before3 = v3r.clone();
    let before4 = v4r.clone();
    let r3 = v3r.apply_delta(&unwrap_v3(&frame));
    let r4 = v4r.apply_delta(&frame);
    assert_eq!(r3, r4, "the two replicas disagree on {frame:?}");
    assert_eq!(v3r, v4r.v3(), "the v3 state differs after {frame:?}");
    assert!(v4r.positions().is_empty(), "a position appeared");
    assert_eq!(v4r.sample_clock(), None);
    if r4.is_err() {
        assert_eq!(*v3r, before3, "the v3 replica changed on a refusal");
        assert_eq!(*v4r, before4, "the v4 replica changed on a refusal");
    }
    r4
}

fn step_publication(
    v3r: &mut ReplicaWorldV3,
    v4r: &mut ReplicaWorldV4,
    publication: &PublicationV3,
) -> Result<(), ReplicaError> {
    let before3 = v3r.clone();
    let before4 = v4r.clone();
    let r3 = v3r.apply_snapshot(publication);
    let r4 = v4r.apply_snapshot(publication);
    assert_eq!(r3, r4, "the two replicas disagree on a publication");
    assert_eq!(v3r, v4r.v3(), "the v3 state differs after a publication");
    assert!(v4r.positions().is_empty(), "a position appeared");
    if r4.is_err() {
        assert_eq!(*v3r, before3);
        assert_eq!(*v4r, before4);
    }
    r4
}

fn ev(n: u64, tick: u64, ops: Vec<V3Op>) -> DeltaFrame {
    frame(n, tick, ops.into_iter().map(DeltaOp::V3).collect())
}

fn v3enter(n: u64) -> V3Op {
    V3Op::EntityEnter { entity: id(n) }
}

fn v3leave(n: u64) -> V3Op {
    V3Op::EntityLeave { entity: id(n) }
}

#[test]
fn r2_matches_the_v3_replica() {
    let mut v3r = ReplicaWorldV3::new(SessionEpoch(Z16));
    let mut v4r = replica();
    let unknown = |n| Err(ReplicaError::UnknownEntity { entity: id(n) });

    // 1 — one mixed legal frame.
    let step1 = vec![
        v3enter(1),
        v3enter(2),
        V3Op::Health {
            entity: id(1),
            health: 3,
            max_health: 10,
            dead: false,
        },
        V3Op::Turn {
            active: Some(id(1)),
            round: 1,
        },
        V3Op::ActionResolved {
            actor: id(1),
            target: id(2),
            ability: u(1),
            outcome: "success".to_string(),
            damage: 0,
        },
        V3Op::TurnStarted {
            actor: id(1),
            round: 1,
        },
        V3Op::Receipt {
            epoch: Z16,
            lane: 0,
            seq: 1,
            processed_tick: 0,
            status: V3Status::Applied,
        },
        V3Op::Spawned { entity: id(2) },
        V3Op::Died { entity: id(2) },
    ];
    assert_eq!(step(&mut v3r, &mut v4r, ev(1, 10, step1)), Ok(()));
    assert_eq!(v4r.v3().next_event_seq(), 2);
    assert_eq!(v4r.v3().server_tick(), 10);
    // 2 — a stale sequence.
    assert_eq!(
        step(&mut v3r, &mut v4r, ev(1, 11, vec![])),
        Err(ReplicaError::EventSeq {
            expected: 2,
            got: 1
        })
    );
    // 3 — a foreign lane.
    let mut bad_lane = ev(2, 2, vec![]);
    bad_lane.lane = 1;
    assert_eq!(
        step(&mut v3r, &mut v4r, bad_lane),
        Err(ReplicaError::WrongLane { got: 1 })
    );
    // 4 — one op over the frame bound.
    let too_many = vec![V3Op::Spawned { entity: id(1) }; MAX_DELTA_OPS + 1];
    assert_eq!(
        step(&mut v3r, &mut v4r, ev(2, 2, too_many)),
        Err(ReplicaError::TooManyOps { got: 257 })
    );
    // 5 — health for an unknown id.
    let health9 = V3Op::Health {
        entity: id(9),
        health: 1,
        max_health: 1,
        dead: false,
    };
    assert_eq!(
        step(&mut v3r, &mut v4r, ev(2, 2, vec![health9.clone()])),
        unknown(9)
    );
    // 6 — a duplicate enter.
    assert_eq!(
        step(&mut v3r, &mut v4r, ev(2, 2, vec![v3enter(1)])),
        Err(ReplicaError::DuplicateEntity { entity: id(1) })
    );
    // 7 — a leave.
    assert_eq!(
        step(&mut v3r, &mut v4r, ev(2, 12, vec![v3leave(2)])),
        Ok(())
    );
    // 8 — a re-enter of a retired id.
    assert_eq!(
        step(&mut v3r, &mut v4r, ev(3, 3, vec![v3enter(2)])),
        Err(ReplicaError::ReusedEntityId { entity: id(2) })
    );
    // 9 — a receipt of another epoch.
    let foreign_receipt = V3Op::Receipt {
        epoch: [1; 16],
        lane: 0,
        seq: 1,
        processed_tick: 0,
        status: V3Status::Applied,
    };
    assert_eq!(
        step(&mut v3r, &mut v4r, ev(3, 3, vec![foreign_receipt])),
        Err(ReplicaError::WrongEpoch)
    );
    // 10 — an encounter end.
    let ended = V3Op::EncounterEnded {
        encounter: u(3),
        round: 1,
    };
    assert_eq!(step(&mut v3r, &mut v4r, ev(3, 13, vec![ended])), Ok(()));
    // 11 — a publication.
    let body = SnapshotBodyV3 {
        entities: vec![
            SnapshotEntityV3 {
                entity: id(1),
                health: Some(crpg_net::protocol_transfer::HealthFact {
                    health: 5,
                    max_health: 10,
                    dead: false,
                }),
                facts: FactSet::default(),
            },
            SnapshotEntityV3 {
                entity: id(3),
                health: None,
                facts: FactSet::default(),
            },
        ],
        turn: Some(crpg_net::protocol_transfer::TurnFact {
            active: Some(id(3)),
            round: 2,
        }),
        encounter: EncounterFacts::default(),
        timeline: None,
    };
    assert_eq!(
        step_publication(&mut v3r, &mut v4r, &snap(3, 20, body)),
        Ok(())
    );
    assert_eq!(v4r.v3().next_event_seq(), 4);
    assert_eq!(v4r.v3().server_tick(), 20);
    // 12 — a re-enter of a retired id.
    assert_eq!(
        step(&mut v3r, &mut v4r, ev(4, 4, vec![v3enter(2)])),
        Err(ReplicaError::ReusedEntityId { entity: id(2) })
    );
    // 13 — a fresh enter and a health.
    let health4 = V3Op::Health {
        entity: id(4),
        health: 1,
        max_health: 1,
        dead: true,
    };
    assert_eq!(
        step(&mut v3r, &mut v4r, ev(4, 21, vec![v3enter(4), health4])),
        Ok(())
    );
    // 14 — a publication below the last applied sequence.
    assert_eq!(
        step_publication(&mut v3r, &mut v4r, &snap(2, 20, e0())),
        Err(ReplicaError::SnapshotCursor { min: 4, got: 2 })
    );
    // 15 — notices for an unknown id, each in its own frame.
    for op in [
        V3Op::Spawned { entity: id(9) },
        V3Op::Died { entity: id(9) },
        v3leave(9),
        V3Op::Despawned { entity: id(9) },
    ] {
        assert_eq!(step(&mut v3r, &mut v4r, ev(5, 5, vec![op])), unknown(9));
    }
    // 16 — a turn naming an unknown id.
    let turn9 = V3Op::Turn {
        active: Some(id(9)),
        round: 3,
    };
    assert_eq!(step(&mut v3r, &mut v4r, ev(5, 5, vec![turn9])), unknown(9));
    // 17 — an action resolution: actor first, then target.
    let resolved = |actor, target| V3Op::ActionResolved {
        actor: id(actor),
        target: id(target),
        ability: u(1),
        outcome: "success".to_string(),
        damage: 0,
    };
    assert_eq!(
        step(&mut v3r, &mut v4r, ev(5, 5, vec![resolved(9, 8)])),
        unknown(9)
    );
    assert_eq!(
        step(&mut v3r, &mut v4r, ev(5, 5, vec![resolved(1, 8)])),
        unknown(8)
    );
    // 18 — a turn start naming an unknown id.
    let started9 = V3Op::TurnStarted {
        actor: id(9),
        round: 3,
    };
    assert_eq!(
        step(&mut v3r, &mut v4r, ev(5, 5, vec![started9])),
        unknown(9)
    );
    // 19 — a despawn of a live row.
    assert_eq!(
        step(
            &mut v3r,
            &mut v4r,
            ev(5, 22, vec![V3Op::Despawned { entity: id(4) }])
        ),
        Ok(())
    );
    // 20 — a second despawn.
    assert_eq!(
        step(
            &mut v3r,
            &mut v4r,
            ev(6, 6, vec![V3Op::Despawned { entity: id(4) }])
        ),
        unknown(4)
    );
    // 21 — fill the replica to its row cap.
    let mut next = 5;
    for seq in 6..=13u64 {
        let count = if seq == 13 { 254 } else { 256 };
        let ops: Vec<V3Op> = (next..next + count).map(v3enter).collect();
        next += count;
        assert_eq!(step(&mut v3r, &mut v4r, ev(seq, seq, ops)), Ok(()));
    }
    assert_eq!(next, 2_051);
    assert_eq!(v4r.v3().len(), 2_048);
    // 22 — one row over the cap.
    assert_eq!(
        step(&mut v3r, &mut v4r, ev(14, 14, vec![v3enter(2_051)])),
        Err(ReplicaError::TooManyEntities)
    );
    // 23 — a leave makes room for the enter.
    assert_eq!(
        step(
            &mut v3r,
            &mut v4r,
            ev(14, 14, vec![v3leave(1), v3enter(2_051)])
        ),
        Ok(())
    );
    assert_eq!(v4r.v3().len(), 2_048);
    // 24 — a publication at the last sequence closes the session.
    assert_eq!(
        step_publication(&mut v3r, &mut v4r, &snap(SEQ_LAST, 30, e0())),
        Ok(())
    );
    assert_eq!(v4r.v3().next_event_seq(), u64::MAX);
    // 25 — nothing more is accepted.
    assert_eq!(
        step(&mut v3r, &mut v4r, ev(u64::MAX, 31, vec![])),
        Err(ReplicaError::EventSeq {
            expected: u64::MAX,
            got: u64::MAX
        })
    );
    // The control for the step-4 frame: it is refused for its length alone.
    let mut fresh3 = ReplicaWorldV3::new(SessionEpoch(Z16));
    let mut fresh4 = replica();
    assert_eq!(
        step(&mut fresh3, &mut fresh4, ev(1, 1, vec![v3enter(1); 1])),
        Ok(())
    );
    let at_bound = vec![V3Op::Spawned { entity: id(1) }; MAX_DELTA_OPS];
    assert_eq!(step(&mut fresh3, &mut fresh4, ev(2, 2, at_bound)), Ok(()));
}

#[test]
fn r2_multi_fault_frames_keep_the_v3_precedence() {
    // Lane 1, 257 ops and a wrong event_seq: the lane wins.
    let many = vec![V3Op::Spawned { entity: id(1) }; MAX_DELTA_OPS + 1];
    let mut lane1 = ev(7, 7, many.clone());
    lane1.lane = 1;
    // Lane 0, 257 ops and a wrong event_seq: the count wins.
    let too_many = ev(7, 7, many);
    // A wrong event_seq with an unknown-entity op: the sequence wins.
    let health9 = ev(
        7,
        7,
        vec![V3Op::Health {
            entity: id(9),
            health: 1,
            max_health: 1,
            dead: false,
        }],
    );
    let expected = [
        (lane1, ReplicaError::WrongLane { got: 1 }),
        (too_many, ReplicaError::TooManyOps { got: 257 }),
        (
            health9,
            ReplicaError::EventSeq {
                expected: 1,
                got: 7,
            },
        ),
    ];
    for (frame, error) in expected {
        let mut v3r = ReplicaWorldV3::new(SessionEpoch(Z16));
        let mut v4r = replica();
        assert_eq!(step(&mut v3r, &mut v4r, frame), Err(error));
    }

    // On v4 alone: a wrong event_seq with a dangling position.
    let mut v4r = replica();
    refused(
        &mut v4r,
        &f(7, 7, vec![pos(9, a(), false)]),
        ReplicaError::EventSeq {
            expected: 1,
            got: 7,
        },
    );
    // Lane 1 with a v4-only op, and 257 v4-only ops, keep the same order.
    let mut lane1 = f(7, 7, vec![moved(9, vec![a(), b()])]);
    lane1.lane = 1;
    refused(&mut v4r, &lane1, ReplicaError::WrongLane { got: 1 });
    refused(
        &mut v4r,
        &f(7, 7, vec![forget(9); MAX_DELTA_OPS + 1]),
        ReplicaError::TooManyOps { got: 257 },
    );
    // `event_seq` above `SEQ_LAST` is an EventSeq even when it is expected.
    refused(
        &mut v4r,
        &f(SEQ_LAST + 1, 7, vec![]),
        ReplicaError::EventSeq {
            expected: 1,
            got: SEQ_LAST + 1,
        },
    );
    // Controls: the right sequence reaches the op, and a dangling position
    // there is `UnknownEntity`, not `EventSeq`.
    refused(
        &mut v4r,
        &f(1, 7, vec![pos(9, a(), false)]),
        ReplicaError::UnknownEntity { entity: id(9) },
    );
    assert_eq!(v4r.apply_delta(&f(1, 7, vec![])), Ok(()));
    assert_eq!(v4r.v3().next_event_seq(), 2);
}

// ---------------------------------------------------------------------------
// R3 — anchors and removal.
// ---------------------------------------------------------------------------

#[test]
fn r3_anchors_and_removal() {
    let mut replica = anchored();
    let expected: BTreeMap<NetId, PositionState> = [
        (id(1), state(100, a(), false)),
        (id(2), state(100, b(), true)),
    ]
    .into();
    assert_eq!(replica.positions(), &expected);
    assert_eq!(replica.position(id(1)), Some(&state(100, a(), false)));
    assert_eq!(replica.position(id(2)), Some(&state(100, b(), true)));
    assert_eq!(replica.position(id(3)), None);
    assert_eq!(replica.v3().next_event_seq(), 2);
    assert_eq!(replica.v3().server_tick(), 100);
    assert_eq!(replica.v3().len(), 2);

    // A `None` removes one position and keeps the row.
    assert_eq!(replica.apply_delta(&f(2, 101, vec![forget(1)])), Ok(()));
    assert_eq!(replica.position(id(1)), None);
    assert_eq!(replica.position(id(2)), Some(&state(100, b(), true)));
    assert!(replica.v3().entity(id(1)).is_some());

    // A leave removes the row and its position.
    assert_eq!(replica.apply_delta(&f(3, 102, vec![leave(2)])), Ok(()));
    assert_eq!(replica.positions(), &BTreeMap::new());
    assert!(replica.v3().entity(id(2)).is_none());

    // A dangling position, with a value or a forget, is refused.
    refused(
        &mut replica,
        &f(4, 103, vec![pos(9, a(), false)]),
        ReplicaError::UnknownEntity { entity: id(9) },
    );
    refused(
        &mut replica,
        &f(4, 103, vec![forget(9)]),
        ReplicaError::UnknownEntity { entity: id(9) },
    );
    // So is a position for the row that just left.
    refused(
        &mut replica,
        &f(4, 103, vec![pos(2, a(), false)]),
        ReplicaError::UnknownEntity { entity: id(2) },
    );
    refused(
        &mut replica,
        &f(4, 103, vec![forget(2)]),
        ReplicaError::UnknownEntity { entity: id(2) },
    );

    // Control: a position in the frame that enters the row, then a despawn of
    // it, is accepted and leaves no position.
    assert_eq!(
        replica.apply_delta(&f(4, 103, vec![enter(3), pos(3, a(), false), despawned(3)])),
        Ok(())
    );
    assert_eq!(replica.positions(), &BTreeMap::new());
    assert!(replica.v3().entity(id(3)).is_none());
    assert_eq!(replica.v3().next_event_seq(), 5);
    assert_eq!(replica.v3().server_tick(), 103);

    // A forget for a live row with no position is `Ok` and changes nothing
    // but the sequence and the tick.
    assert_eq!(replica.apply_delta(&f(5, 104, vec![forget(1)])), Ok(()));
    assert_eq!(replica.positions(), &BTreeMap::new());
    assert!(replica.v3().entity(id(1)).is_some());
    assert_eq!(replica.v3().next_event_seq(), 6);
    assert_eq!(replica.v3().server_tick(), 104);

    // An enter, a position and a leave in one frame leave no position.
    assert_eq!(
        replica.apply_delta(&f(6, 105, vec![enter(4), pos(4, b(), true), leave(4)])),
        Ok(())
    );
    assert_eq!(replica.positions(), &BTreeMap::new());

    // A position for a live row is stored; the invariant holds.
    assert_eq!(
        replica.apply_delta(&f(7, 106, vec![pos(1, b(), true)])),
        Ok(())
    );
    assert_eq!(replica.position(id(1)), Some(&state(106, b(), true)));
    for key in replica.positions().keys() {
        assert!(replica.v3().entity(*key).is_some());
    }
}

// ---------------------------------------------------------------------------
// R4 — `Moved` is an unchecked notice.
// ---------------------------------------------------------------------------

#[test]
fn r4_moved_is_an_unchecked_notice() {
    let mut replica = replica();
    let mut oracle = ReplicaWorldV3::new(SessionEpoch(Z16));
    assert_eq!(replica.apply_delta(&f(1, 100, vec![enter(1)])), Ok(()));
    assert_eq!(
        oracle.apply_delta(&V3Frame {
            lane: 0,
            server_tick: 100,
            event_seq: 1,
            ops: vec![v3enter(1)],
        }),
        Ok(())
    );
    assert_eq!(replica.v3(), &oracle);
    assert!(replica.positions().is_empty());

    // A live entity, an entity that was never live, and one that enters later
    // in the same frame: each is `Ok`.
    let snapshot_of = |replica: &ReplicaWorldV4| replica.clone();
    let before = snapshot_of(&replica);
    assert_eq!(
        replica.apply_delta(&f(2, 101, vec![moved(1, vec![a(), b()])])),
        Ok(())
    );
    assert_eq!(replica.v3().next_event_seq(), 3);
    assert_eq!(replica.v3().server_tick(), 101);
    assert_eq!(replica.v3().entities(), before.v3().entities());
    assert!(replica.positions().is_empty());
    assert_eq!(replica.sample_clock(), None);

    assert_eq!(
        replica.apply_delta(&f(3, 102, vec![moved(9, vec![a(), b()])])),
        Ok(())
    );
    assert_eq!(replica.v3().next_event_seq(), 4);
    assert_eq!(replica.v3().server_tick(), 102);
    assert_eq!(replica.v3().len(), 1);
    assert!(replica.positions().is_empty());
    assert!(replica.v3().entity(id(9)).is_none());

    assert_eq!(
        replica.apply_delta(&f(4, 103, vec![moved(5, vec![a(), b()]), enter(5)])),
        Ok(())
    );
    assert_eq!(replica.v3().next_event_seq(), 5);
    assert_eq!(replica.v3().server_tick(), 103);
    assert_eq!(
        replica.v3().entities(),
        &[
            ReplicaEntityV3 {
                net_id: id(1),
                health: None,
                facts: FactSet::default(),
            },
            ReplicaEntityV3 {
                net_id: id(5),
                health: None,
                facts: FactSet::default(),
            },
        ]
    );
    assert!(replica.positions().is_empty());
    assert_eq!(replica.position(id(1)), None);
    assert_eq!(replica.position(id(5)), None);

    // Control: a `Moved` beside a held position changes no position.
    assert_eq!(
        replica.apply_delta(&f(5, 104, vec![pos(1, a(), true)])),
        Ok(())
    );
    assert_eq!(
        replica.apply_delta(&f(6, 105, vec![moved(1, vec![b(), a()])])),
        Ok(())
    );
    assert_eq!(replica.position(id(1)), Some(&state(104, a(), true)));
}

// ---------------------------------------------------------------------------
// R5 — samples, newest wins.
// ---------------------------------------------------------------------------

#[test]
fn r5_samples_newest_wins() {
    let mut replica = anchored();
    assert_eq!(replica.apply_delta(&f(2, 101, vec![enter(3)])), Ok(()));
    let v3_before = replica.v3().clone();

    // Equal to the stored tick: not newer, stores nothing; the clock starts.
    let one = sample(100, 1, vec![entry(1, p(1, 0, 0), true)]);
    assert_eq!(replica.apply_sample(&one), 0);
    assert_eq!(replica.position(id(1)), Some(&state(100, a(), false)));
    assert_eq!(replica.sample_clock(), clock(100, 1));
    assert_eq!(replica.v3(), &v3_before);

    // Newer: 1 and 2 stored, 3 undisclosed, 9 unknown.
    let two = sample(
        101,
        2,
        vec![
            entry(1, p(1, 0, 0), true),
            entry(2, p(2, 0, 0), true),
            entry(3, a(), false),
            entry(9, a(), false),
        ],
    );
    assert_eq!(replica.apply_sample(&two), 2);
    assert_eq!(replica.position(id(1)), Some(&state(101, p(1, 0, 0), true)));
    assert_eq!(replica.position(id(2)), Some(&state(101, p(2, 0, 0), true)));
    assert_eq!(replica.position(id(3)), None);
    assert_eq!(replica.position(id(9)), None);
    assert_eq!(replica.positions().len(), 2);
    assert_eq!(replica.sample_clock(), clock(101, 2));
    assert_eq!(replica.v3(), &v3_before);

    // Older: stores nothing and the clock does not move back.
    let three = sample(99, 3, vec![entry(1, a(), false)]);
    assert_eq!(replica.apply_sample(&three), 0);
    assert_eq!(replica.position(id(1)), Some(&state(101, p(1, 0, 0), true)));
    assert_eq!(replica.sample_clock(), clock(101, 2));
    assert_eq!(replica.v3(), &v3_before);

    // An empty sample is newer for the clock and stores nothing.
    assert_eq!(replica.apply_sample(&sample(102, 4, vec![])), 0);
    assert_eq!(replica.sample_clock(), clock(102, 4));
    assert_eq!(replica.position(id(2)), Some(&state(101, p(2, 0, 0), true)));

    // A sample with a repeated entity stores the first and skips the second
    // (not newer than what it just stored).
    let twice = sample(
        103,
        5,
        vec![entry(2, p(5, 0, 0), false), entry(2, p(6, 0, 0), true)],
    );
    assert_eq!(replica.apply_sample(&twice), 1);
    assert_eq!(
        replica.position(id(2)),
        Some(&state(103, p(5, 0, 0), false))
    );
}

// ---------------------------------------------------------------------------
// R6 — a snapshot clears positions.
// ---------------------------------------------------------------------------

#[test]
fn r6_snapshot_clears_positions() {
    let mut replica = anchored();
    assert_eq!(
        replica.apply_sample(&sample(101, 2, vec![entry(1, p(1, 0, 0), true)])),
        1
    );
    assert_eq!(replica.sample_clock(), clock(101, 2));
    assert_eq!(replica.positions().len(), 2);

    // A refused publication changes nothing, positions included.
    let before = replica.clone();
    assert_eq!(
        replica.apply_snapshot(&snap(0, 50, rows(&[1, 2]))),
        Err(ReplicaError::SnapshotCursor { min: 1, got: 0 })
    );
    assert_eq!(replica, before);
    assert_eq!(replica.positions().len(), 2);
    // The same for a body that fails a later check (rows out of order).
    assert_eq!(
        replica.apply_snapshot(&snap(1, 50, rows(&[2, 1]))),
        Err(ReplicaError::SnapshotOrder { entity: id(1) })
    );
    assert_eq!(replica, before);

    // The control: cursor 1 is accepted.
    assert_eq!(replica.apply_snapshot(&snap(1, 50, rows(&[1, 2]))), Ok(()));
    assert_eq!(replica.positions(), &BTreeMap::new());
    assert_eq!(replica.position(id(1)), None);
    assert_eq!(replica.sample_clock(), clock(101, 2));
    let mut oracle = ReplicaWorldV3::new(SessionEpoch(Z16));
    assert_eq!(
        oracle.apply_delta(&unwrap_v3(&f(1, 100, vec![enter(1), enter(2)]))),
        Ok(())
    );
    assert_eq!(oracle.apply_snapshot(&snap(1, 50, rows(&[1, 2]))), Ok(()));
    assert_eq!(replica.v3(), &oracle);
    assert_eq!(replica.v3().next_event_seq(), 2);
    assert_eq!(replica.v3().server_tick(), 50);

    // The next page re-anchors, and the invariant holds throughout.
    assert_eq!(
        replica.apply_delta(&f(2, 102, vec![pos(1, b(), true)])),
        Ok(())
    );
    assert_eq!(replica.position(id(1)), Some(&state(102, b(), true)));
    for key in replica.positions().keys() {
        assert!(replica.v3().entity(*key).is_some());
    }
    // A snapshot that drops a row drops its position, with the others.
    assert_eq!(replica.apply_snapshot(&snap(2, 60, rows(&[2]))), Ok(()));
    assert_eq!(replica.positions(), &BTreeMap::new());
    assert!(replica.v3().entity(id(1)).is_none());
}

// ---------------------------------------------------------------------------
// R7 — a mid-frame failure changes nothing.
// ---------------------------------------------------------------------------

#[test]
fn r7_mid_frame_failure_changes_nothing() {
    let mut replica = anchored();
    refused(
        &mut replica,
        &f(2, 101, vec![pos(1, b(), true), leave(2), forget(9)]),
        ReplicaError::UnknownEntity { entity: id(9) },
    );
    assert_eq!(replica.position(id(1)), Some(&state(100, a(), false)));
    assert_eq!(replica.position(id(2)), Some(&state(100, b(), true)));
    assert!(replica.v3().entity(id(2)).is_some());
    assert_eq!(replica.v3().next_event_seq(), 2);
    assert_eq!(replica.v3().server_tick(), 100);

    // A v3 fault after a position change, and the other order, also leak
    // nothing.
    refused(
        &mut replica,
        &f(2, 101, vec![forget(1), enter(1)]),
        ReplicaError::DuplicateEntity { entity: id(1) },
    );
    refused(
        &mut replica,
        &f(
            2,
            101,
            vec![enter(3), pos(3, a(), true), forget(2), pos(9, a(), true)],
        ),
        ReplicaError::UnknownEntity { entity: id(9) },
    );
    assert!(replica.v3().entity(id(3)).is_none());

    // The control: the same frame without the bad op is accepted whole.
    assert_eq!(
        replica.apply_delta(&f(2, 101, vec![pos(1, b(), true), leave(2)])),
        Ok(())
    );
    assert_eq!(replica.position(id(1)), Some(&state(101, b(), true)));
    assert_eq!(replica.position(id(2)), None);
    assert!(replica.v3().entity(id(2)).is_none());
}

// ---------------------------------------------------------------------------
// R8 — an anchor against a newer sample.
// ---------------------------------------------------------------------------

#[test]
fn r8_anchor_versus_newer_sample() {
    let mut replica = anchored();
    assert_eq!(
        replica.apply_sample(&sample(105, 1, vec![entry(1, p(1, 0, 0), true)])),
        1
    );
    assert_eq!(replica.position(id(1)), Some(&state(105, p(1, 0, 0), true)));
    assert_eq!(replica.sample_clock(), clock(105, 1));

    // An equal tick: stored nothing, and the clock stays on its first sample.
    assert_eq!(
        replica.apply_sample(&sample(105, 7, vec![entry(1, p(3, 0, 0), true)])),
        0
    );
    assert_eq!(replica.position(id(1)), Some(&state(105, p(1, 0, 0), true)));
    assert_eq!(replica.sample_clock(), clock(105, 1));

    // An older anchor is kept out.
    assert_eq!(
        replica.apply_delta(&f(2, 104, vec![pos(1, a(), false)])),
        Ok(())
    );
    assert_eq!(replica.position(id(1)), Some(&state(105, p(1, 0, 0), true)));
    assert_eq!(replica.v3().next_event_seq(), 3);
    assert_eq!(replica.v3().server_tick(), 104);

    // An equal-tick anchor replaces.
    assert_eq!(
        replica.apply_delta(&f(3, 105, vec![pos(1, a(), false)])),
        Ok(())
    );
    assert_eq!(replica.position(id(1)), Some(&state(105, a(), false)));

    // A newer anchor replaces.
    assert_eq!(
        replica.apply_delta(&f(4, 106, vec![pos(1, b(), true)])),
        Ok(())
    );
    assert_eq!(replica.position(id(1)), Some(&state(106, b(), true)));

    // The other entity's anchor (tick 100) was never touched by any of it.
    assert_eq!(replica.position(id(2)), Some(&state(100, b(), true)));

    // An older anchor for an entity with no position still stores it.
    assert_eq!(replica.apply_delta(&f(5, 90, vec![forget(2)])), Ok(()));
    assert_eq!(
        replica.apply_delta(&f(6, 80, vec![pos(2, a(), false)])),
        Ok(())
    );
    assert_eq!(replica.position(id(2)), Some(&state(80, a(), false)));
}

// ---------------------------------------------------------------------------
// R9 — pool to replica, end to end.
// ---------------------------------------------------------------------------

#[test]
fn r9_pool_to_replica_end_to_end() {
    let mut pool = ReassemblyPoolV4::new(TransferLimitsV3::v3()).expect("v3 limits");
    let receiver = pool.add_receiver(Z16, GateMode::Live).expect("receiver");

    let d1 = f(1, 100, vec![enter(1), pos(1, a(), false)]);
    let d2 = f(2, 101, vec![moved(1, vec![a(), b()])]);
    let d3 = f(3, 120, vec![pos(1, b(), true)]);
    let d1_bytes = encode_delta(&d1).expect("d1");
    let d2_bytes = encode_delta(&d2).expect("d2");
    let d3_bytes = encode_delta(&d3).expect("d3");
    let body = rows(&[1]);
    let plan = plan_transfer(Z16, tid(1), 10, 2, 8, &body).expect("fixture plan");

    assert_eq!(
        pool.offer_delta(receiver, &d1_bytes, 1_000),
        Ok(DeltaOutcome::Released { count: 1 })
    );
    assert_eq!(
        pool.offer_delta(receiver, &d2_bytes, 1_000),
        Ok(DeltaOutcome::Released { count: 1 })
    );
    assert_eq!(plan.len(), 2);
    assert_eq!(
        pool.offer_transfer(receiver, &plan[0], 1_000),
        Ok(TransferOutcome::Begun)
    );
    let published = pool
        .offer_transfer(receiver, &plan[1], 1_000)
        .expect("snapshot completes");
    assert!(
        matches!(
            published,
            TransferOutcome::Published {
                transfer,
                ..
            } if transfer == tid(1)
        ),
        "unexpected outcome {published:?}"
    );
    assert_eq!(
        pool.offer_delta(receiver, &d3_bytes, 1_000),
        Ok(DeltaOutcome::Released { count: 1 })
    );

    let publication = snap(2, 10, body);
    let mut updates = Vec::new();
    while let Some(update) = pool.pop_update(receiver) {
        updates.push(update);
    }
    assert_eq!(
        updates,
        vec![
            ReplicaUpdateV4::Delta(d1.clone()),
            ReplicaUpdateV4::Delta(d2.clone()),
            ReplicaUpdateV4::Snapshot(publication.clone()),
            ReplicaUpdateV4::Delta(d3.clone()),
        ]
    );

    let mut through_pool = replica();
    for update in &updates {
        match update {
            ReplicaUpdateV4::Snapshot(publication) => {
                assert_eq!(through_pool.apply_snapshot(publication), Ok(()));
            }
            ReplicaUpdateV4::Delta(frame) => {
                assert_eq!(through_pool.apply_delta(frame), Ok(()));
            }
        }
    }
    let mut direct = replica();
    assert_eq!(direct.apply_delta(&d1), Ok(()));
    assert_eq!(direct.apply_delta(&d2), Ok(()));
    assert_eq!(direct.apply_snapshot(&publication), Ok(()));
    assert_eq!(direct.apply_delta(&d3), Ok(()));

    let expected: BTreeMap<NetId, PositionState> = [(id(1), state(120, b(), true))].into();
    assert_eq!(through_pool.positions(), &expected);
    assert_eq!(direct.positions(), &expected);
    assert_eq!(through_pool, direct);
    assert_eq!(through_pool.v3().next_event_seq(), 4);
    assert_eq!(through_pool.v3().server_tick(), 120);
    assert_eq!(through_pool.sample_clock(), None);
}

// ---------------------------------------------------------------------------
// Outbox.
// ---------------------------------------------------------------------------

fn halt_intent(seq: u64, actor: u64) -> MoveIntent {
    MoveIntent {
        epoch: Z16,
        seq,
        actor: id(actor),
        body: MoveBody::Halt,
    }
}

fn receipt(seq: u64, actor: u64, outcome: MoveOutcome) -> MoveReceipt {
    MoveReceipt {
        epoch: Z16,
        seq,
        actor: id(actor),
        processed_tick: 100,
        outcome,
    }
}

/// An outbox with eight `Halt` intents for actor 1 pending.
fn full_outbox() -> MoveOutbox {
    let mut outbox = MoveOutbox::new(Z16);
    for seq in 1..=8u64 {
        assert_eq!(outbox.send(id(1), MoveBody::Halt), Ok(halt_intent(seq, 1)));
    }
    outbox
}

#[test]
fn o1_send_window() {
    let mut outbox = MoveOutbox::new(Z16);
    assert_eq!(outbox.epoch(), Z16);
    assert_eq!(outbox.next_seq(), 1);
    assert_eq!(outbox.processed_seq(), 0);
    assert_eq!(outbox.pending(), &[]);
    assert_eq!(POLICY_MOVE_IN_FLIGHT, 8);

    let mut sent = Vec::new();
    for seq in 1..=8u64 {
        let intent = outbox.send(id(1), MoveBody::Halt).expect("inside window");
        assert_eq!(intent, halt_intent(seq, 1));
        sent.push(intent);
        assert_eq!(outbox.next_seq(), seq + 1);
        assert_eq!(outbox.pending(), &sent[..]);
    }
    let before = outbox.clone();
    assert_eq!(outbox.send(id(1), MoveBody::Halt), Err(OutboxError::Full));
    assert_eq!(outbox, before);
    assert_eq!(outbox.next_seq(), 9);
    assert_eq!(outbox.pending(), &sent[..]);
    assert_eq!(outbox.processed_seq(), 0);

    // The body and the actor are the caller's, and the epoch is the outbox's.
    let mut other = MoveOutbox::new([5; 16]);
    let to = MoveBody::MoveTo { destination: b() };
    assert_eq!(
        other.send(id(3), to),
        Ok(MoveIntent {
            epoch: [5; 16],
            seq: 1,
            actor: id(3),
            body: to,
        })
    );
    assert_eq!(other.epoch(), [5; 16]);
}

#[test]
fn o2_receive_in_order() {
    let mut outbox = full_outbox();
    assert_eq!(
        outbox.receive(&receipt(1, 1, MoveOutcome::Accepted { start_tick: 101 })),
        Ok(())
    );
    assert_eq!(outbox.processed_seq(), 1);
    assert_eq!(outbox.pending().len(), 7);
    assert_eq!(
        outbox.receive(&receipt(2, 1, MoveOutcome::Superseded)),
        Ok(())
    );
    assert_eq!(outbox.processed_seq(), 2);
    assert_eq!(
        outbox.receive(&receipt(
            3,
            1,
            MoveOutcome::Rejected(RejectionCode::IllegalAction)
        )),
        Ok(())
    );
    assert_eq!(outbox.processed_seq(), 3);
    let expected: Vec<MoveIntent> = (4..=8).map(|seq| halt_intent(seq, 1)).collect();
    assert_eq!(outbox.pending(), &expected[..]);
    assert_eq!(outbox.next_seq(), 9);

    // The slots freed by the receipts reopen the window.
    assert_eq!(outbox.send(id(1), MoveBody::Halt), Ok(halt_intent(9, 1)));
    assert_eq!(outbox.next_seq(), 10);
    assert_eq!(outbox.pending().len(), 6);
    assert_eq!(outbox.pending()[5], halt_intent(9, 1));
    // The window is full again at eight pending.
    assert_eq!(outbox.send(id(1), MoveBody::Halt), Ok(halt_intent(10, 1)));
    assert_eq!(outbox.send(id(1), MoveBody::Halt), Ok(halt_intent(11, 1)));
    assert_eq!(outbox.pending().len(), 8);
    assert_eq!(outbox.send(id(1), MoveBody::Halt), Err(OutboxError::Full));
    // A resume resends `pending()`: ascending `seq`.
    let seqs: Vec<u64> = outbox.pending().iter().map(|intent| intent.seq).collect();
    assert_eq!(seqs, vec![4, 5, 6, 7, 8, 9, 10, 11]);
}

#[test]
fn o3_receive_refusals() {
    let mut outbox = full_outbox();
    for (seq, outcome) in [
        (1, MoveOutcome::Accepted { start_tick: 101 }),
        (2, MoveOutcome::Superseded),
        (3, MoveOutcome::Rejected(RejectionCode::IllegalAction)),
    ] {
        assert_eq!(outbox.receive(&receipt(seq, 1, outcome)), Ok(()));
    }
    assert_eq!(outbox.send(id(1), MoveBody::Halt), Ok(halt_intent(9, 1)));

    let accepted = MoveOutcome::Accepted { start_tick: 102 };
    let mut foreign = receipt(4, 1, accepted);
    foreign.epoch = [1; 16];
    let cases = [
        (foreign, OutboxError::WrongEpoch),
        (
            receipt(5, 1, accepted),
            OutboxError::Unexpected {
                expected: Some(4),
                got: 5,
            },
        ),
        (
            receipt(3, 1, accepted),
            OutboxError::Unexpected {
                expected: Some(4),
                got: 3,
            },
        ),
        (receipt(4, 2, accepted), OutboxError::WrongActor { seq: 4 }),
        // The sequence is checked before the actor.
        (
            receipt(5, 2, accepted),
            OutboxError::Unexpected {
                expected: Some(4),
                got: 5,
            },
        ),
    ];
    for (bad, error) in cases {
        let before = outbox.clone();
        assert_eq!(outbox.receive(&bad), Err(error), "{bad:?}");
        assert_eq!(outbox, before, "a refused receipt changed the outbox");
    }
    // A foreign epoch with the wrong seq and actor is still the epoch.
    let mut worse = receipt(9, 2, accepted);
    worse.epoch = [1; 16];
    assert_eq!(outbox.receive(&worse), Err(OutboxError::WrongEpoch));

    // The control: the right receipt is accepted.
    assert_eq!(outbox.receive(&receipt(4, 1, accepted)), Ok(()));
    assert_eq!(outbox.processed_seq(), 4);
    assert_eq!(outbox.pending().first(), Some(&halt_intent(5, 1)));

    // A fresh outbox: nothing pending.
    let mut fresh = MoveOutbox::new(Z16);
    let before = fresh.clone();
    assert_eq!(
        fresh.receive(&receipt(1, 1, accepted)),
        Err(OutboxError::Unexpected {
            expected: None,
            got: 1
        })
    );
    assert_eq!(fresh, before);
    let mut foreign = receipt(1, 1, accepted);
    foreign.epoch = [1; 16];
    assert_eq!(fresh.receive(&foreign), Err(OutboxError::WrongEpoch));
    assert_eq!(fresh, before);
    // Control: with one pending intent the same receipt is accepted.
    assert_eq!(fresh.send(id(1), MoveBody::Halt), Ok(halt_intent(1, 1)));
    assert_eq!(fresh.receive(&receipt(1, 1, accepted)), Ok(()));
    assert_eq!(fresh.pending(), &[]);
    assert_eq!(fresh.processed_seq(), 1);
    // Receipts are for the oldest only: a repeat is now unexpected.
    assert_eq!(
        fresh.receive(&receipt(1, 1, accepted)),
        Err(OutboxError::Unexpected {
            expected: None,
            got: 1
        })
    );
    assert_eq!(SEQ_LAST, u64::MAX - 1);
}

#[test]
fn o4_outbox_error_text() {
    let cases = [
        (OutboxError::Exhausted, "Exhausted at movement/outbox"),
        (OutboxError::Full, "Full at movement/outbox"),
        (OutboxError::WrongEpoch, "WrongEpoch at movement/outbox"),
        (
            OutboxError::Unexpected {
                expected: Some(4),
                got: 5,
            },
            "Unexpected at movement/outbox",
        ),
        (
            OutboxError::Unexpected {
                expected: None,
                got: 1,
            },
            "Unexpected at movement/outbox",
        ),
        (
            OutboxError::WrongActor { seq: 4 },
            "WrongActor at movement/outbox",
        ),
    ];
    for (error, text) in cases {
        assert_eq!(error.to_string(), text);
        assert!(error.source().is_none());
    }
    // The payload never reaches the text.
    assert_eq!(
        OutboxError::Unexpected {
            expected: None,
            got: 1
        }
        .to_string(),
        "Unexpected at movement/outbox"
    );
    assert_eq!(
        OutboxError::WrongActor { seq: 99 }.to_string(),
        "WrongActor at movement/outbox"
    );
}

// ---------------------------------------------------------------------------
// Review fix round: survivors of the mutant run.
// ---------------------------------------------------------------------------

#[test]
fn r2_event_seq_above_last_wins_over_an_op_error() {
    let mut replica = anchored();
    // A publication at SEQ_LAST leaves `next_event_seq` at u64::MAX, so a frame
    // at u64::MAX equals the expected sequence but is above SEQ_LAST.
    assert_eq!(
        replica.apply_snapshot(&snap(SEQ_LAST, 50, rows(&[1, 2]))),
        Ok(())
    );
    assert_eq!(replica.v3().next_event_seq(), u64::MAX);
    let expected = ReplicaError::EventSeq {
        expected: u64::MAX,
        got: u64::MAX,
    };
    // A faulty v4 op, a faulty v3 op and a valid op: all are EventSeq.
    refused(
        &mut replica,
        &f(u64::MAX, 51, vec![pos(9, a(), false)]),
        expected,
    );
    refused(&mut replica, &f(u64::MAX, 51, vec![forget(9)]), expected);
    refused(&mut replica, &f(u64::MAX, 51, vec![leave(9)]), expected);
    refused(
        &mut replica,
        &f(u64::MAX, 51, vec![pos(1, b(), true)]),
        expected,
    );
    refused(&mut replica, &f(u64::MAX, 51, vec![]), expected);
}

#[test]
fn r4_moved_changes_no_held_position() {
    let mut replica = replica();
    assert_eq!(
        replica.apply_delta(&f(1, 100, vec![enter(1), pos(1, a(), false)])),
        Ok(())
    );
    let held = state(100, a(), false);
    assert_eq!(replica.position(id(1)), Some(&held));
    // The route's last point is not the held point, and the route is a real
    // move, so storing a point or flipping `moving` would show.
    let far = p(3 * 65_536, 65_536, 7 * 65_536);
    assert_eq!(
        replica.apply_delta(&f(2, 101, vec![moved(1, vec![b(), far])])),
        Ok(())
    );
    assert_eq!(replica.position(id(1)), Some(&held));
    assert_eq!(replica.positions().len(), 1);
    // The same in a frame that also carries a newer anchor for another row.
    assert_eq!(
        replica.apply_delta(&f(
            3,
            102,
            vec![enter(2), moved(1, vec![far, b()]), pos(2, b(), true)]
        )),
        Ok(())
    );
    assert_eq!(replica.position(id(1)), Some(&held));
    assert_eq!(replica.position(id(2)), Some(&state(102, b(), true)));
}

#[test]
fn r4_only_leave_and_despawn_remove_a_position() {
    let mut replica = anchored();
    let v3health = V3Op::Health {
        entity: id(1),
        health: 3,
        max_health: 10,
        dead: false,
    };
    let held1 = state(100, a(), false);
    let held2 = state(100, b(), true);
    for (n, op) in [
        v3(V3Op::Died { entity: id(1) }),
        v3(V3Op::Spawned { entity: id(1) }),
        v3(v3health),
    ]
    .into_iter()
    .enumerate()
    {
        let seq = 2 + n as u64;
        assert_eq!(replica.apply_delta(&f(seq, 100 + seq, vec![op])), Ok(()));
        assert_eq!(replica.position(id(1)), Some(&held1), "op {n}");
        assert_eq!(replica.position(id(2)), Some(&held2), "op {n}");
    }
    // Controls: a despawn removes row 1's position and a leave row 2's.
    assert_eq!(replica.apply_delta(&f(5, 105, vec![despawned(1)])), Ok(()));
    assert_eq!(replica.position(id(1)), None);
    assert_eq!(replica.position(id(2)), Some(&held2));
    assert_eq!(replica.apply_delta(&f(6, 106, vec![leave(2)])), Ok(()));
    assert_eq!(replica.positions(), &BTreeMap::new());
}
