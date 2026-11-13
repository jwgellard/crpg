//! T140r2 implementer suite: the protocol v3 client replica (`replica_v3`),
//! derived from `tasks/T140r.md` Behaviour R-1..R-15 and the tests R1..R10,
//! through the public API only.
//!
//! Expectations are hand-written whole values: whole results, full error
//! values and whole replica state through the public accessors. Every refusal
//! is checked to leave the replica equal to its pre-call clone, and each one
//! has a passing control. R2 runs T046's `ReplicaWorld` side by side as the
//! differential oracle. R10 drives a `ReassemblyPoolV3` end to end.

use crpg_core::Ulid;
use crpg_net::codec_v3::encode_delta;
use crpg_net::pool_v3::{
    DeltaOutcome, PublicationV3, ReassemblyPoolV3, ReplicaUpdateV3, TransferOutcome,
};
use crpg_net::protocol::{
    DeltaOp as V1Op, ReceiptStatus, MAX_DELTA_OPS, MAX_VISIBLE_ENTITIES, SEQ_LAST,
};
use crpg_net::protocol_transfer::{HealthFact, SnapshotBody, SnapshotEntity, TransferId, TurnFact};
use crpg_net::protocol_v2::{DeltaFrame as V2Frame, DeltaOp as V2Op};
use crpg_net::protocol_v3::{
    AutoPauseToggles, AutoPauseTrigger, ClockFact, DeltaFrame, DeltaOp, EncounterFacts,
    EncounterMode, FactSet, HostFacts, NetId, OrderFact, PauseReason, PauseRights, PoolFact,
    SessionEpoch, StatFact, StatValue, TimelineEntry,
};
use crpg_net::reassembly::GateMode;
use crpg_net::reassembly_v3::TransferLimitsV3;
use crpg_net::replica::{ReplicaError, ReplicaWorld, MAX_REPLICA_ENTITIES};
use crpg_net::replica_v3::{PauseState, ReplicaEntityV3, ReplicaWorldV3};
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

fn replica() -> ReplicaWorldV3 {
    ReplicaWorldV3::new(SessionEpoch(Z16))
}

fn frame(seq: u64, tick: u64, ops: Vec<DeltaOp>) -> DeltaFrame {
    DeltaFrame {
        lane: 0,
        server_tick: tick,
        event_seq: seq,
        ops,
    }
}

/// The contract's `F(n, ops)`: tick and seq `n`.
fn f(seq: u64, ops: Vec<DeltaOp>) -> DeltaFrame {
    frame(seq, seq, ops)
}

fn enter(n: u64) -> DeltaOp {
    DeltaOp::EntityEnter { entity: id(n) }
}

fn leave(n: u64) -> DeltaOp {
    DeltaOp::EntityLeave { entity: id(n) }
}

fn turn(active: Option<u64>, round: u64) -> DeltaOp {
    DeltaOp::Turn {
        active: active.map(id),
        round,
    }
}

fn encounter(facts: EncounterFacts) -> DeltaOp {
    DeltaOp::Encounter(facts)
}

fn timeline(entries: Option<Vec<TimelineEntry>>) -> DeltaOp {
    DeltaOp::Timeline { entries }
}

fn entity_facts(entity: u64, facts: FactSet) -> DeltaOp {
    DeltaOp::EntityFacts {
        entity: id(entity),
        facts,
    }
}

fn tl(key: i32, entity: u64) -> TimelineEntry {
    TimelineEntry {
        key,
        entity: id(entity),
    }
}

fn ord(target: u64) -> Option<Option<OrderFact>> {
    Some(Some(OrderFact {
        ability: u(2),
        target: id(target),
        repeat: true,
    }))
}

fn fact(health: u32, max_health: u32, dead: bool) -> HealthFact {
    HealthFact {
        health,
        max_health,
        dead,
    }
}

fn hf(health: u32, max_health: u32, dead: bool) -> Option<HealthFact> {
    Some(fact(health, max_health, dead))
}

fn e0() -> SnapshotBodyV3 {
    SnapshotBodyV3 {
        entities: vec![],
        turn: None,
        encounter: EncounterFacts::default(),
        timeline: None,
    }
}

fn ef_a() -> EncounterFacts {
    EncounterFacts {
        mode: Some(EncounterMode::RealTime),
        pending_mode: Some(Some(EncounterMode::TurnBased)),
        clock: Some(Some(ClockFact {
            round_ticks: 60,
            elapsed: 5,
        })),
    }
}

fn host_facts_h() -> HostFacts {
    HostFacts {
        rights: PauseRights {
            pause: true,
            unpause: false,
        },
        auto_pause: Some(AutoPauseToggles::from_bits(0x05).expect("masked bits")),
        turn_timer_ms: Some(None),
    }
}

fn rowof(net: u64, facts: FactSet) -> SnapshotEntityV3 {
    SnapshotEntityV3 {
        entity: id(net),
        health: None,
        facts,
    }
}

fn body(
    entities: Vec<SnapshotEntityV3>,
    turn: Option<TurnFact>,
    encounter: EncounterFacts,
    timeline: Option<Vec<TimelineEntry>>,
) -> SnapshotBodyV3 {
    SnapshotBodyV3 {
        entities,
        turn,
        encounter,
        timeline,
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

fn row(net: u64, health: Option<HealthFact>, facts: FactSet) -> ReplicaEntityV3 {
    ReplicaEntityV3 {
        net_id: id(net),
        health,
        facts,
    }
}

// R2's differential oracle helpers.

fn v2frame(seq: u64, tick: u64, ops: Vec<V2Op>) -> V2Frame {
    V2Frame {
        lane: 0,
        server_tick: tick,
        event_seq: seq,
        ops,
    }
}

fn convert(frame: &V2Frame) -> DeltaFrame {
    DeltaFrame {
        lane: frame.lane,
        server_tick: frame.server_tick,
        event_seq: frame.event_seq,
        ops: frame.ops.iter().cloned().map(DeltaOp::from).collect(),
    }
}

fn check_both(v2: &ReplicaWorld, v3: &ReplicaWorldV3) {
    assert_eq!(
        v2.entities()
            .iter()
            .map(|row| (row.net_id, row.health.clone()))
            .collect::<Vec<_>>(),
        v3.entities()
            .iter()
            .map(|row| (row.net_id, row.health.clone()))
            .collect::<Vec<_>>()
    );
    assert_eq!(v2.turn(), v3.turn());
    assert_eq!(v2.next_event_seq(), v3.next_event_seq());
    assert_eq!(v2.server_tick(), v3.server_tick());
    for row in v3.entities() {
        assert_eq!(row.facts, FactSet::default());
    }
}

/// R4's "unchanged": the six public state fields, not the sequence counters.
fn same_visible_state(left: &ReplicaWorldV3, right: &ReplicaWorldV3) -> bool {
    left.entities() == right.entities()
        && left.turn() == right.turn()
        && left.encounter() == right.encounter()
        && left.timeline() == right.timeline()
        && left.pause() == right.pause()
        && left.host_facts() == right.host_facts()
}

/// Applies one frame to both replicas and asserts that they agree.
fn step(
    v2: &mut ReplicaWorld,
    v3: &mut ReplicaWorldV3,
    frame: V2Frame,
) -> Result<(), ReplicaError> {
    let before_v2 = v2.clone();
    let before_v3 = v3.clone();
    let v3frame = convert(&frame);
    let result_v2 = v2.apply_delta(&frame);
    let result_v3 = v3.apply_delta(&v3frame);
    assert_eq!(result_v2, result_v3);
    if result_v2.is_err() {
        assert_eq!(v2, &before_v2);
        assert_eq!(v3, &before_v3);
    }
    check_both(v2, v3);
    result_v2
}

/// Applies one publication to both replicas and asserts that they agree.
fn step_publication(
    v2: &mut ReplicaWorld,
    v3: &mut ReplicaWorldV3,
    cursor: u64,
    base: u64,
    rows: Vec<(u64, Option<HealthFact>)>,
    turn: Option<TurnFact>,
) -> Result<(), ReplicaError> {
    let before_v2 = v2.clone();
    let before_v3 = v3.clone();
    let v2pub = crpg_net::reassembly::Publication {
        transfer: tid(1),
        base_tick: base,
        cursor,
        body: SnapshotBody {
            entities: rows
                .iter()
                .map(|(entity, health)| SnapshotEntity {
                    entity: id(*entity),
                    health: health.clone(),
                })
                .collect(),
            turn: turn.clone(),
        },
    };
    let v3pub = snap(
        cursor,
        base,
        body(
            rows.iter()
                .map(|(entity, health)| SnapshotEntityV3 {
                    entity: id(*entity),
                    health: health.clone(),
                    facts: FactSet::default(),
                })
                .collect(),
            turn,
            EncounterFacts::default(),
            None,
        ),
    );
    let result_v2 = v2.apply_snapshot(&v2pub);
    let result_v3 = v3.apply_snapshot(&v3pub);
    assert_eq!(result_v2, result_v3);
    if result_v2.is_err() {
        assert_eq!(v2, &before_v2);
        assert_eq!(v3, &before_v3);
    }
    check_both(v2, v3);
    result_v2
}

/// Asserts a refusal leaves the replica equal to its pre-call clone.
fn refused(replica: &mut ReplicaWorldV3, frame: &DeltaFrame, expected: ReplicaError) {
    let before = (*replica).clone();
    assert_eq!(replica.apply_delta(frame), Err(expected));
    assert_eq!(*replica, before);
}

fn snapshot_refused(
    replica: &mut ReplicaWorldV3,
    publication: &PublicationV3,
    expected: ReplicaError,
) {
    let before = (*replica).clone();
    assert_eq!(replica.apply_snapshot(publication), Err(expected));
    assert_eq!(*replica, before);
}

// ---------------------------------------------------------------------------
// R1 — construction.
// ---------------------------------------------------------------------------

#[test]
fn r1_construction() {
    let replica = replica();
    assert_eq!(replica.epoch(), SessionEpoch(Z16));
    assert_eq!(replica.entities(), &[]);
    assert_eq!(replica.len(), 0);
    assert!(replica.is_empty());
    assert_eq!(replica.entity(id(1)), None);
    assert_eq!(replica.turn(), None);
    assert_eq!(replica.encounter(), &EncounterFacts::default());
    assert_eq!(replica.timeline(), None);
    assert_eq!(replica.pause(), None);
    assert_eq!(replica.host_facts(), None);
    assert_eq!(replica.next_event_seq(), 1);
    assert_eq!(replica.server_tick(), 0);
    assert_eq!(replica, ReplicaWorldV3::new(SessionEpoch(Z16)));
    // The epoch never changes, and it is not part of any accepted input.
    let mut moved = replica.clone();
    assert_eq!(moved.apply_delta(&f(1, vec![enter(1)])), Ok(()));
    assert_eq!(moved.epoch(), SessionEpoch(Z16));
    assert_ne!(moved, replica);
    assert_ne!(ReplicaWorldV3::new(SessionEpoch([7; 16])), replica);
}

// ---------------------------------------------------------------------------
// R2 — the v2 replica oracle.
// ---------------------------------------------------------------------------

#[test]
fn r2_matches_the_v2_replica() {
    let mut v2 = ReplicaWorld::new(SessionEpoch(Z16));
    let mut v3 = replica();

    let step1 = vec![
        V2Op::Legacy(V1Op::EntityEnter { entity: id(1) }),
        V2Op::Legacy(V1Op::EntityEnter { entity: id(2) }),
        V2Op::Legacy(V1Op::Health {
            entity: id(1),
            health: 3,
            max_health: 10,
            dead: false,
        }),
        V2Op::Legacy(V1Op::Turn {
            active: Some(id(1)),
            round: 1,
        }),
        V2Op::ActionResolved {
            actor: id(1),
            target: id(2),
            ability: u(1),
            outcome: "success".to_string(),
            damage: 0,
        },
        V2Op::TurnStarted {
            actor: id(1),
            round: 1,
        },
        V2Op::Legacy(V1Op::Receipt {
            epoch: Z16,
            lane: 0,
            seq: 1,
            processed_tick: 0,
            status: ReceiptStatus::Applied,
        }),
        V2Op::Legacy(V1Op::Spawned { entity: id(2) }),
        V2Op::Legacy(V1Op::Died { entity: id(2) }),
    ];
    // 1 — one mixed legal frame.
    assert_eq!(step(&mut v2, &mut v3, v2frame(1, 10, step1)), Ok(()));
    // 2 — a stale sequence.
    assert_eq!(
        step(&mut v2, &mut v3, v2frame(1, 11, vec![])),
        Err(ReplicaError::EventSeq {
            expected: 2,
            got: 1
        })
    );
    // 3 — a foreign lane.
    let mut bad_lane = v2frame(2, 2, vec![]);
    bad_lane.lane = 1;
    assert_eq!(
        step(&mut v2, &mut v3, bad_lane),
        Err(ReplicaError::WrongLane { got: 1 })
    );
    // 4 — one op over the frame bound.
    let too_many = vec![V2Op::Legacy(V1Op::Spawned { entity: id(1) }); MAX_DELTA_OPS + 1];
    assert_eq!(
        step(&mut v2, &mut v3, v2frame(2, 2, too_many)),
        Err(ReplicaError::TooManyOps { got: 257 })
    );
    // 5 — health for an unknown id.
    assert_eq!(
        step(
            &mut v2,
            &mut v3,
            v2frame(
                2,
                2,
                vec![V2Op::Legacy(V1Op::Health {
                    entity: id(9),
                    health: 1,
                    max_health: 1,
                    dead: false
                })]
            )
        ),
        Err(ReplicaError::UnknownEntity { entity: id(9) })
    );
    // 6 — a duplicate enter.
    assert_eq!(
        step(
            &mut v2,
            &mut v3,
            v2frame(
                2,
                2,
                vec![V2Op::Legacy(V1Op::EntityEnter { entity: id(1) })]
            )
        ),
        Err(ReplicaError::DuplicateEntity { entity: id(1) })
    );
    // 7 — a legal leave.
    assert_eq!(
        step(
            &mut v2,
            &mut v3,
            v2frame(
                2,
                12,
                vec![V2Op::Legacy(V1Op::EntityLeave { entity: id(2) })]
            )
        ),
        Ok(())
    );
    // 8 — a retired id cannot re-enter.
    assert_eq!(
        step(
            &mut v2,
            &mut v3,
            v2frame(
                3,
                3,
                vec![V2Op::Legacy(V1Op::EntityEnter { entity: id(2) })]
            )
        ),
        Err(ReplicaError::ReusedEntityId { entity: id(2) })
    );
    // 9 — a foreign receipt epoch.
    assert_eq!(
        step(
            &mut v2,
            &mut v3,
            v2frame(
                3,
                3,
                vec![V2Op::Legacy(V1Op::Receipt {
                    epoch: [1; 16],
                    lane: 0,
                    seq: 1,
                    processed_tick: 0,
                    status: ReceiptStatus::Applied
                })]
            )
        ),
        Err(ReplicaError::WrongEpoch)
    );
    // 10 — a legal encounter end.
    assert_eq!(
        step(
            &mut v2,
            &mut v3,
            v2frame(
                3,
                13,
                vec![V2Op::EncounterEnded {
                    encounter: u(3),
                    round: 1
                }]
            )
        ),
        Ok(())
    );
    // 11 — a publication with two rows.
    assert_eq!(
        step_publication(
            &mut v2,
            &mut v3,
            3,
            20,
            vec![(1, hf(5, 10, false)), (3, None)],
            Some(TurnFact {
                active: Some(id(3)),
                round: 2
            })
        ),
        Ok(())
    );
    // 12 — a body row below the high-water mark.
    assert_eq!(
        step(
            &mut v2,
            &mut v3,
            v2frame(
                4,
                4,
                vec![V2Op::Legacy(V1Op::EntityEnter { entity: id(2) })]
            )
        ),
        Err(ReplicaError::ReusedEntityId { entity: id(2) })
    );
    // 13 — a legal enter with health.
    assert_eq!(
        step(
            &mut v2,
            &mut v3,
            v2frame(
                4,
                21,
                vec![
                    V2Op::Legacy(V1Op::EntityEnter { entity: id(4) }),
                    V2Op::Legacy(V1Op::Health {
                        entity: id(4),
                        health: 1,
                        max_health: 1,
                        dead: true
                    })
                ]
            )
        ),
        Ok(())
    );
    // 14 — a cursor behind the last applied sequence.
    assert_eq!(
        step_publication(&mut v2, &mut v3, 2, 0, vec![], None),
        Err(ReplicaError::SnapshotCursor { min: 4, got: 2 })
    );
    // 15 — every notice for a removed id.
    for op in [
        V2Op::Legacy(V1Op::Spawned { entity: id(9) }),
        V2Op::Legacy(V1Op::Died { entity: id(9) }),
        V2Op::Legacy(V1Op::EntityLeave { entity: id(9) }),
        V2Op::Legacy(V1Op::Despawned { entity: id(9) }),
    ] {
        assert_eq!(
            step(&mut v2, &mut v3, v2frame(5, 5, vec![op])),
            Err(ReplicaError::UnknownEntity { entity: id(9) })
        );
    }
    // 16 — an unknown turn active.
    assert_eq!(
        step(
            &mut v2,
            &mut v3,
            v2frame(
                5,
                5,
                vec![V2Op::Legacy(V1Op::Turn {
                    active: Some(id(9)),
                    round: 3
                })]
            )
        ),
        Err(ReplicaError::UnknownEntity { entity: id(9) })
    );
    // 17 — the actor, then the target.
    assert_eq!(
        step(
            &mut v2,
            &mut v3,
            v2frame(
                5,
                5,
                vec![V2Op::ActionResolved {
                    actor: id(9),
                    target: id(8),
                    ability: u(1),
                    outcome: "success".to_string(),
                    damage: 0
                }]
            )
        ),
        Err(ReplicaError::UnknownEntity { entity: id(9) })
    );
    assert_eq!(
        step(
            &mut v2,
            &mut v3,
            v2frame(
                5,
                5,
                vec![V2Op::ActionResolved {
                    actor: id(1),
                    target: id(8),
                    ability: u(1),
                    outcome: "success".to_string(),
                    damage: 0
                }]
            )
        ),
        Err(ReplicaError::UnknownEntity { entity: id(8) })
    );
    // 18 — an unknown turn head.
    assert_eq!(
        step(
            &mut v2,
            &mut v3,
            v2frame(
                5,
                5,
                vec![V2Op::TurnStarted {
                    actor: id(9),
                    round: 3
                }]
            )
        ),
        Err(ReplicaError::UnknownEntity { entity: id(9) })
    );
    // 19 — a legal despawn.
    assert_eq!(
        step(
            &mut v2,
            &mut v3,
            v2frame(5, 22, vec![V2Op::Legacy(V1Op::Despawned { entity: id(4) })])
        ),
        Ok(())
    );
    // 20 — a second despawn is unknown.
    assert_eq!(
        step(
            &mut v2,
            &mut v3,
            v2frame(6, 6, vec![V2Op::Legacy(V1Op::Despawned { entity: id(4) })])
        ),
        Err(ReplicaError::UnknownEntity { entity: id(4) })
    );
    // 21 — fill the replica to its mid-page bound.
    let mut raw = 5u64;
    for seq in 6..=12u64 {
        let ops = (0..MAX_DELTA_OPS)
            .map(|_| {
                let entity = raw;
                raw += 1;
                V2Op::Legacy(V1Op::EntityEnter { entity: id(entity) })
            })
            .collect();
        assert_eq!(step(&mut v2, &mut v3, v2frame(seq, seq, ops)), Ok(()));
    }
    let ops = (0..254)
        .map(|_| {
            let entity = raw;
            raw += 1;
            V2Op::Legacy(V1Op::EntityEnter { entity: id(entity) })
        })
        .collect();
    assert_eq!(step(&mut v2, &mut v3, v2frame(13, 13, ops)), Ok(()));
    assert_eq!(v3.len(), MAX_REPLICA_ENTITIES);
    assert_eq!(raw, 2051);
    // 22 — one over the bound.
    assert_eq!(
        step(
            &mut v2,
            &mut v3,
            v2frame(
                14,
                14,
                vec![V2Op::Legacy(V1Op::EntityEnter { entity: id(2051) })]
            )
        ),
        Err(ReplicaError::TooManyEntities)
    );
    // 23 — a leave makes room again.
    assert_eq!(
        step(
            &mut v2,
            &mut v3,
            v2frame(
                14,
                14,
                vec![
                    V2Op::Legacy(V1Op::EntityLeave { entity: id(1) }),
                    V2Op::Legacy(V1Op::EntityEnter { entity: id(2051) })
                ]
            )
        ),
        Ok(())
    );
    assert_eq!(v3.len(), MAX_REPLICA_ENTITIES);
    // 24 — a publication at the last sequence.
    assert_eq!(
        step_publication(&mut v2, &mut v3, SEQ_LAST, 30, vec![], None),
        Ok(())
    );
    assert_eq!(v3.next_event_seq(), u64::MAX);
    // 25 — nothing is accepted after it.
    assert_eq!(
        step(&mut v2, &mut v3, v2frame(u64::MAX, u64::MAX, vec![])),
        Err(ReplicaError::EventSeq {
            expected: u64::MAX,
            got: u64::MAX
        })
    );
}

// ---------------------------------------------------------------------------
// R3 — fact images replace whole.
// ---------------------------------------------------------------------------

#[test]
fn r3_fact_ops_replace_images() {
    let mut replica = replica();
    assert_eq!(replica.apply_delta(&f(1, vec![enter(1), enter(2)])), Ok(()));
    assert_eq!(
        replica.entity(id(1)),
        Some(&row(1, None, FactSet::default()))
    );
    assert_eq!(
        replica.entity(id(2)),
        Some(&row(2, None, FactSet::default()))
    );

    let fs_a = FactSet {
        pools: Some(vec![PoolFact {
            pool: u(1),
            current: 3,
            max: 5,
        }]),
        effects: Some(vec![]),
        order: ord(2),
        stats: None,
        tags: Some(vec!["elite".to_string()]),
    };
    assert_eq!(
        replica.apply_delta(&f(2, vec![entity_facts(1, fs_a.clone())])),
        Ok(())
    );
    assert_eq!(replica.entity(id(1)), Some(&row(1, None, fs_a)));
    assert_eq!(
        replica.entity(id(2)),
        Some(&row(2, None, FactSet::default()))
    );

    let only_order = FactSet {
        order: Some(None),
        ..FactSet::default()
    };
    assert_eq!(
        replica.apply_delta(&f(3, vec![entity_facts(1, only_order.clone())])),
        Ok(())
    );
    assert_eq!(replica.entity(id(1)), Some(&row(1, None, only_order)));

    assert_eq!(
        replica.apply_delta(&f(4, vec![entity_facts(1, FactSet::default())])),
        Ok(())
    );
    assert_eq!(
        replica.entity(id(1)),
        Some(&row(1, None, FactSet::default()))
    );

    assert_eq!(replica.apply_delta(&f(5, vec![encounter(ef_a())])), Ok(()));
    assert_eq!(replica.encounter(), &ef_a());

    let turn_only = EncounterFacts {
        mode: Some(EncounterMode::TurnBased),
        ..EncounterFacts::default()
    };
    assert_eq!(
        replica.apply_delta(&f(6, vec![encounter(turn_only)])),
        Ok(())
    );
    assert_eq!(replica.encounter(), &turn_only);

    assert_eq!(
        replica.apply_delta(&f(7, vec![encounter(EncounterFacts::default())])),
        Ok(())
    );
    assert_eq!(replica.encounter(), &EncounterFacts::default());

    let entries = vec![tl(3, 2), tl(3, 1)];
    assert_eq!(
        replica.apply_delta(&f(8, vec![timeline(Some(entries.clone()))])),
        Ok(())
    );
    assert_eq!(replica.timeline(), Some(entries.as_slice()));

    assert_eq!(
        replica.apply_delta(&f(9, vec![timeline(Some(vec![]))])),
        Ok(())
    );
    assert_eq!(replica.timeline(), Some(&[][..]));

    assert_eq!(replica.apply_delta(&f(10, vec![timeline(None)])), Ok(()));
    assert_eq!(replica.timeline(), None);

    assert_eq!(
        replica.apply_delta(&f(
            11,
            vec![entity_facts(
                2,
                FactSet {
                    order: ord(1),
                    ..FactSet::default()
                }
            )]
        )),
        Ok(())
    );
    assert_eq!(
        replica.entity(id(2)),
        Some(&row(
            2,
            None,
            FactSet {
                order: ord(1),
                ..FactSet::default()
            }
        ))
    );

    assert_eq!(replica.apply_delta(&f(12, vec![leave(2)])), Ok(()));
    assert_eq!(replica.entity(id(2)), None);

    assert_eq!(replica.apply_delta(&f(13, vec![enter(3)])), Ok(()));
    assert_eq!(
        replica.entity(id(3)),
        Some(&row(3, None, FactSet::default()))
    );
}

// ---------------------------------------------------------------------------
// R4 — fact references are live; notices are inert.
// ---------------------------------------------------------------------------

#[test]
fn r4_fact_references_live_and_notices_inert() {
    let mut replica = replica();
    assert_eq!(replica.apply_delta(&f(1, vec![enter(1), enter(2)])), Ok(()));

    let unmatched_target = FactSet {
        order: ord(9),
        ..FactSet::default()
    };
    refused(
        &mut replica,
        &f(2, vec![entity_facts(9, FactSet::default())]),
        ReplicaError::UnknownEntity { entity: id(9) },
    );
    refused(
        &mut replica,
        &f(2, vec![entity_facts(1, unmatched_target.clone())]),
        ReplicaError::UnknownEntity { entity: id(9) },
    );
    refused(
        &mut replica,
        &f(2, vec![entity_facts(8, unmatched_target)]),
        ReplicaError::UnknownEntity { entity: id(8) },
    );
    refused(
        &mut replica,
        &f(2, vec![timeline(Some(vec![tl(0, 1), tl(1, 9), tl(2, 8)]))]),
        ReplicaError::UnknownEntity { entity: id(9) },
    );
    refused(
        &mut replica,
        &f(2, vec![entity_facts(4, FactSet::default()), enter(4)]),
        ReplicaError::UnknownEntity { entity: id(4) },
    );

    // 1 — a fully legal fact/turn/encounter frame.
    assert_eq!(
        replica.apply_delta(&f(
            2,
            vec![
                enter(3),
                entity_facts(
                    3,
                    FactSet {
                        order: ord(3),
                        ..FactSet::default()
                    }
                ),
                timeline(Some(vec![tl(0, 3)])),
                turn(Some(3), 4),
                encounter(ef_a()),
            ]
        )),
        Ok(())
    );
    assert_eq!(replica.len(), 3);

    // 2 — notices are accepted and change nothing, for live and unknown ids.
    let before = replica.clone();
    assert_eq!(
        replica.apply_delta(&f(
            3,
            vec![
                DeltaOp::OrderEnded { actor: id(3) },
                DeltaOp::Joined { entity: id(3) },
                DeltaOp::Left { entity: id(3) },
                DeltaOp::OrderEnded { actor: id(9) },
                DeltaOp::Joined { entity: id(9) },
                DeltaOp::Left { entity: id(9) },
            ]
        )),
        Ok(())
    );
    assert!(same_visible_state(&replica, &before));
    assert_eq!(replica.len(), 3);
    assert_eq!(
        replica.entity(id(3)),
        Some(&row(
            3,
            None,
            FactSet {
                order: ord(3),
                ..FactSet::default()
            }
        ))
    );

    // 3 — an encounter end changes neither the turn nor the encounter.
    let before = replica.clone();
    assert_eq!(
        replica.apply_delta(&f(
            4,
            vec![
                DeltaOp::RoundStarted { round: 9 },
                DeltaOp::ModeChanged {
                    mode: EncounterMode::TurnBased,
                    round: 9,
                },
                DeltaOp::EncounterEnded {
                    encounter: u(1),
                    round: 9,
                },
            ]
        )),
        Ok(())
    );
    assert!(same_visible_state(&replica, &before));
    assert_eq!(
        replica.turn(),
        Some(&TurnFact {
            active: Some(id(3)),
            round: 4,
        })
    );
    assert_eq!(replica.encounter(), &ef_a());
    assert_eq!(replica.timeline(), Some(&[tl(0, 3)][..]));

    // 4 — a descending, repeated timeline is stored as given.
    let descending = vec![tl(5, 1), tl(4, 1)];
    assert_eq!(
        replica.apply_delta(&f(5, vec![timeline(Some(descending.clone()))])),
        Ok(())
    );
    assert_eq!(replica.timeline(), Some(descending.as_slice()));

    // 5 — a pool with `current > max` is stored as given.
    let over = FactSet {
        pools: Some(vec![PoolFact {
            pool: u(1),
            current: 9,
            max: 3,
        }]),
        ..FactSet::default()
    };
    assert_eq!(
        replica.apply_delta(&f(6, vec![entity_facts(1, over.clone())])),
        Ok(())
    );
    assert_eq!(replica.entity(id(1)), Some(&row(1, None, over)));
}

// ---------------------------------------------------------------------------
// R5 — stored references may go stale.
// ---------------------------------------------------------------------------

#[test]
fn r5_stale_references_are_tolerated() {
    let mut replica = replica();
    assert_eq!(
        replica.apply_delta(&f(
            1,
            vec![
                enter(1),
                enter(2),
                entity_facts(
                    1,
                    FactSet {
                        order: ord(2),
                        ..FactSet::default()
                    }
                ),
                timeline(Some(vec![tl(0, 1), tl(1, 2)])),
                turn(Some(2), 1),
            ]
        )),
        Ok(())
    );

    assert_eq!(replica.apply_delta(&f(2, vec![leave(2)])), Ok(()));
    assert_eq!(replica.entity(id(2)), None);
    assert_eq!(
        replica.entity(id(1)),
        Some(&row(
            1,
            None,
            FactSet {
                order: ord(2),
                ..FactSet::default()
            }
        ))
    );
    assert_eq!(replica.timeline(), Some(&[tl(0, 1), tl(1, 2)][..]));
    assert_eq!(
        replica.turn(),
        Some(&TurnFact {
            active: Some(id(2)),
            round: 1,
        })
    );

    refused(
        &mut replica,
        &f(
            3,
            vec![entity_facts(
                1,
                FactSet {
                    order: ord(2),
                    ..FactSet::default()
                },
            )],
        ),
        ReplicaError::UnknownEntity { entity: id(2) },
    );
}

// ---------------------------------------------------------------------------
// R6 — clock and host facts survive a snapshot.
// ---------------------------------------------------------------------------

#[test]
fn r6_clock_and_host_facts() {
    let mut replica = replica();
    assert_eq!(
        replica.apply_delta(&f(1, vec![DeltaOp::Clock { paused: None }])),
        Ok(())
    );
    assert_eq!(replica.pause(), Some(PauseState::Running));

    assert_eq!(
        replica.apply_delta(&f(2, vec![DeltaOp::Clock { paused: Some(None) }])),
        Ok(())
    );
    assert_eq!(replica.pause(), Some(PauseState::Paused { reason: None }));

    assert_eq!(
        replica.apply_delta(&f(
            3,
            vec![DeltaOp::Clock {
                paused: Some(Some(PauseReason::Auto(AutoPauseTrigger::PartyMemberDied)))
            }]
        )),
        Ok(())
    );
    assert_eq!(
        replica.pause(),
        Some(PauseState::Paused {
            reason: Some(PauseReason::Auto(AutoPauseTrigger::PartyMemberDied))
        })
    );

    let h = host_facts_h();
    assert_eq!(
        replica.apply_delta(&f(4, vec![DeltaOp::HostFacts(h)])),
        Ok(())
    );
    assert_eq!(replica.host_facts(), Some(&h));

    assert_eq!(replica.apply_snapshot(&snap(4, 9, e0())), Ok(()));
    assert_eq!(
        replica.pause(),
        Some(PauseState::Paused {
            reason: Some(PauseReason::Auto(AutoPauseTrigger::PartyMemberDied))
        })
    );
    assert_eq!(replica.host_facts(), Some(&h));
    assert_eq!(replica.next_event_seq(), 5);

    assert_eq!(
        replica.apply_delta(&f(5, vec![DeltaOp::Clock { paused: None }])),
        Ok(())
    );
    assert_eq!(replica.pause(), Some(PauseState::Running));
}

// ---------------------------------------------------------------------------
// R7 — snapshot check order.
// ---------------------------------------------------------------------------

#[test]
fn r7_snapshot_checks_in_order() {
    let mut replica = replica();
    assert_eq!(replica.apply_delta(&f(1, vec![enter(1), enter(2)])), Ok(()));
    assert_eq!(replica.apply_delta(&f(2, vec![enter(4), leave(4)])), Ok(()));
    assert_eq!(replica.next_event_seq(), 3);

    snapshot_refused(
        &mut replica,
        &snap(1, 0, e0()),
        ReplicaError::SnapshotCursor { min: 2, got: 1 },
    );
    snapshot_refused(
        &mut replica,
        &snap(SEQ_LAST + 1, 0, e0()),
        ReplicaError::SnapshotCursor {
            min: 2,
            got: SEQ_LAST + 1,
        },
    );
    let too_many: Vec<_> = (5..=5 + MAX_VISIBLE_ENTITIES as u64)
        .map(|entity| rowof(entity, FactSet::default()))
        .collect();
    snapshot_refused(
        &mut replica,
        &snap(2, 0, body(too_many, None, EncounterFacts::default(), None)),
        ReplicaError::TooManyEntities,
    );
    snapshot_refused(
        &mut replica,
        &snap(
            2,
            0,
            body(
                vec![rowof(2, FactSet::default()), rowof(1, FactSet::default())],
                None,
                EncounterFacts::default(),
                None,
            ),
        ),
        ReplicaError::SnapshotOrder { entity: id(1) },
    );
    snapshot_refused(
        &mut replica,
        &snap(
            2,
            0,
            body(
                vec![rowof(4, FactSet::default())],
                None,
                EncounterFacts::default(),
                None,
            ),
        ),
        ReplicaError::ReusedEntityId { entity: id(4) },
    );
    snapshot_refused(
        &mut replica,
        &snap(
            2,
            0,
            body(
                vec![
                    rowof(
                        1,
                        FactSet {
                            order: ord(9),
                            ..FactSet::default()
                        },
                    ),
                    rowof(2, FactSet::default()),
                ],
                None,
                EncounterFacts::default(),
                None,
            ),
        ),
        ReplicaError::UnknownEntity { entity: id(9) },
    );
    snapshot_refused(
        &mut replica,
        &snap(
            2,
            0,
            body(
                vec![rowof(1, FactSet::default())],
                Some(TurnFact {
                    active: Some(id(2)),
                    round: 1,
                }),
                EncounterFacts::default(),
                None,
            ),
        ),
        ReplicaError::UnknownEntity { entity: id(2) },
    );
    snapshot_refused(
        &mut replica,
        &snap(
            2,
            0,
            body(
                vec![rowof(1, FactSet::default())],
                None,
                EncounterFacts::default(),
                Some(vec![tl(0, 1), tl(0, 2)]),
            ),
        ),
        ReplicaError::UnknownEntity { entity: id(2) },
    );

    // Precedence: reuse before order targets.
    snapshot_refused(
        &mut replica,
        &snap(
            2,
            0,
            body(
                vec![rowof(
                    4,
                    FactSet {
                        order: ord(9),
                        ..FactSet::default()
                    },
                )],
                None,
                EncounterFacts::default(),
                None,
            ),
        ),
        ReplicaError::ReusedEntityId { entity: id(4) },
    );
    // Precedence: order targets before the turn.
    snapshot_refused(
        &mut replica,
        &snap(
            2,
            0,
            body(
                vec![
                    rowof(
                        1,
                        FactSet {
                            order: ord(9),
                            ..FactSet::default()
                        },
                    ),
                    rowof(
                        2,
                        FactSet {
                            order: ord(8),
                            ..FactSet::default()
                        },
                    ),
                ],
                None,
                EncounterFacts::default(),
                None,
            ),
        ),
        ReplicaError::UnknownEntity { entity: id(9) },
    );
    snapshot_refused(
        &mut replica,
        &snap(
            2,
            0,
            body(
                vec![rowof(
                    1,
                    FactSet {
                        order: ord(9),
                        ..FactSet::default()
                    },
                )],
                Some(TurnFact {
                    active: Some(id(8)),
                    round: 1,
                }),
                EncounterFacts::default(),
                None,
            ),
        ),
        ReplicaError::UnknownEntity { entity: id(9) },
    );
    // Precedence: the turn before the timeline.
    snapshot_refused(
        &mut replica,
        &snap(
            2,
            0,
            body(
                vec![rowof(1, FactSet::default())],
                Some(TurnFact {
                    active: Some(id(8)),
                    round: 1,
                }),
                EncounterFacts::default(),
                Some(vec![tl(0, 7)]),
            ),
        ),
        ReplicaError::UnknownEntity { entity: id(8) },
    );

    // Passing control: the same shape, all references inside the body.
    assert_eq!(
        replica.apply_snapshot(&snap(
            2,
            0,
            body(
                vec![rowof(1, FactSet::default()), rowof(2, FactSet::default())],
                Some(TurnFact {
                    active: Some(id(2)),
                    round: 1,
                }),
                EncounterFacts::default(),
                Some(vec![tl(0, 1)]),
            ),
        )),
        Ok(())
    );
    assert_eq!(replica.next_event_seq(), 3);
}

// ---------------------------------------------------------------------------
// R8 — a snapshot replaces everything but host state.
// ---------------------------------------------------------------------------

#[test]
fn r8_snapshot_replaces_everything_but_host_state() {
    let mut replica = replica();
    assert_eq!(
        replica.apply_delta(&f(
            1,
            vec![
                enter(1),
                enter(2),
                enter(3),
                entity_facts(
                    1,
                    FactSet {
                        order: ord(2),
                        ..FactSet::default()
                    }
                ),
                encounter(ef_a()),
                timeline(Some(vec![tl(0, 1)])),
                turn(Some(3), 2),
                DeltaOp::Clock { paused: Some(None) },
            ]
        )),
        Ok(())
    );

    let snapshot = snap(
        5,
        40,
        body(
            vec![
                SnapshotEntityV3 {
                    entity: id(1),
                    health: hf(2, 3, false),
                    facts: FactSet {
                        order: Some(None),
                        ..FactSet::default()
                    },
                },
                rowof(3, FactSet::default()),
                rowof(
                    5,
                    FactSet {
                        stats: Some(vec![StatFact {
                            name: "might".to_string(),
                            value: StatValue::Int(3),
                        }]),
                        ..FactSet::default()
                    },
                ),
            ],
            None,
            EncounterFacts::default(),
            Some(vec![tl(1, 5), tl(1, 1)]),
        ),
    );
    assert_eq!(replica.apply_snapshot(&snapshot), Ok(()));

    assert_eq!(
        replica.entities(),
        &[
            row(
                1,
                hf(2, 3, false),
                FactSet {
                    order: Some(None),
                    ..FactSet::default()
                }
            ),
            row(3, None, FactSet::default()),
            row(
                5,
                None,
                FactSet {
                    stats: Some(vec![StatFact {
                        name: "might".to_string(),
                        value: StatValue::Int(3),
                    }]),
                    ..FactSet::default()
                }
            ),
        ]
    );
    assert_eq!(replica.turn(), None);
    assert_eq!(replica.encounter(), &EncounterFacts::default());
    assert_eq!(replica.timeline(), Some(&[tl(1, 5), tl(1, 1)][..]));
    assert_eq!(replica.pause(), Some(PauseState::Paused { reason: None }));
    assert_eq!(replica.next_event_seq(), 6);
    assert_eq!(replica.server_tick(), 40);
    assert_eq!(replica.entity(id(2)), None);

    refused(
        &mut replica,
        &f(6, vec![enter(2)]),
        ReplicaError::ReusedEntityId { entity: id(2) },
    );
    refused(
        &mut replica,
        &f(6, vec![enter(4)]),
        ReplicaError::ReusedEntityId { entity: id(4) },
    );
    assert_eq!(replica.apply_delta(&f(6, vec![enter(6)])), Ok(()));
    assert_eq!(replica.len(), 4);
}

// ---------------------------------------------------------------------------
// R9 — a failed frame leaves no change.
// ---------------------------------------------------------------------------

#[test]
fn r9_mid_frame_failure_changes_nothing() {
    let mut replica = replica();
    assert_eq!(replica.apply_delta(&f(1, vec![enter(1), enter(2)])), Ok(()));
    let h = host_facts_h();
    let ops = vec![
        entity_facts(
            1,
            FactSet {
                order: ord(2),
                ..FactSet::default()
            },
        ),
        encounter(ef_a()),
        timeline(Some(vec![tl(0, 1)])),
        DeltaOp::Clock { paused: Some(None) },
        DeltaOp::HostFacts(h),
        enter(3),
        leave(1),
        entity_facts(9, FactSet::default()),
    ];
    let before = replica.clone();
    assert_eq!(
        replica.apply_delta(&f(2, ops.clone())),
        Err(ReplicaError::UnknownEntity { entity: id(9) })
    );
    assert_eq!(replica, before);
    assert_eq!(
        replica.entity(id(1)),
        Some(&row(1, None, FactSet::default()))
    );
    assert_eq!(replica.encounter(), &EncounterFacts::default());
    assert_eq!(replica.timeline(), None);
    assert_eq!(replica.pause(), None);
    assert_eq!(replica.host_facts(), None);

    let ok_ops = ops[..ops.len() - 1].to_vec();
    assert_eq!(replica.apply_delta(&f(2, ok_ops)), Ok(()));
    assert_eq!(
        replica.entities(),
        &[
            row(2, None, FactSet::default()),
            row(3, None, FactSet::default())
        ]
    );
    assert_eq!(replica.encounter(), &ef_a());
    assert_eq!(replica.timeline(), Some(&[tl(0, 1)][..]));
    assert_eq!(replica.turn(), None);
    assert_eq!(replica.pause(), Some(PauseState::Paused { reason: None }));
    assert_eq!(replica.host_facts(), Some(&h));
}

// ---------------------------------------------------------------------------
// R10 — pool to replica end to end.
// ---------------------------------------------------------------------------

#[test]
fn r10_pool_to_replica_end_to_end() {
    let bx = body(
        vec![
            SnapshotEntityV3 {
                entity: id(1),
                health: hf(10, 10, false),
                facts: FactSet {
                    pools: Some(vec![PoolFact {
                        pool: u(1),
                        current: 3,
                        max: 5,
                    }]),
                    order: ord(2),
                    ..FactSet::default()
                },
            },
            rowof(2, FactSet::default()),
        ],
        Some(TurnFact {
            active: Some(id(2)),
            round: 3,
        }),
        EncounterFacts {
            mode: Some(EncounterMode::RealTime),
            pending_mode: Some(None),
            clock: Some(Some(ClockFact {
                round_ticks: 60,
                elapsed: 5,
            })),
        },
        Some(vec![tl(1, 2), tl(1, 1)]),
    );

    let mut pool = ReassemblyPoolV3::new(TransferLimitsV3::v3()).expect("v3 limits");
    let receiver = pool
        .add_receiver(Z16, GateMode::AwaitSnapshot)
        .expect("receiver");

    let d1 = encode_delta(&DeltaFrame {
        lane: 0,
        server_tick: 1,
        event_seq: 1,
        ops: vec![],
    })
    .expect("delta 1");
    let seq3 = frame(
        3,
        51,
        vec![
            entity_facts(1, FactSet::default()),
            DeltaOp::Clock {
                paused: Some(Some(PauseReason::Player)),
            },
        ],
    );
    let seq3_bytes = encode_delta(&seq3).expect("delta 3");
    let seq4 = frame(4, 52, vec![leave(2), timeline(Some(vec![tl(1, 1)]))]);
    let seq4_bytes = encode_delta(&seq4).expect("delta 4");

    assert_eq!(
        pool.offer_delta(receiver, &d1, 1_000),
        Ok(DeltaOutcome::Buffered)
    );
    assert_eq!(
        pool.offer_delta(receiver, &seq3_bytes, 1_000),
        Ok(DeltaOutcome::Buffered)
    );

    let plan = plan_transfer(Z16, tid(1), 50, 2, 8, &bx).expect("fixture plan");
    assert_eq!(
        pool.offer_transfer(receiver, &plan[0], 1_000),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        pool.offer_transfer(receiver, &plan[1], 1_000),
        Ok(TransferOutcome::Published {
            transfer: tid(1),
            released: 1,
        })
    );
    assert_eq!(
        pool.offer_delta(receiver, &seq4_bytes, 1_000),
        Ok(DeltaOutcome::Released { count: 1 })
    );

    let updates = vec![
        pool.pop_update(receiver),
        pool.pop_update(receiver),
        pool.pop_update(receiver),
    ];
    assert_eq!(
        updates,
        vec![
            Some(ReplicaUpdateV3::Snapshot(PublicationV3 {
                transfer: tid(1),
                base_tick: 50,
                cursor: 2,
                body: bx.clone(),
            })),
            Some(ReplicaUpdateV3::Delta(seq3.clone())),
            Some(ReplicaUpdateV3::Delta(seq4.clone())),
        ]
    );
    assert_eq!(pool.pop_update(receiver), None);

    let mut replica = replica();
    for update in updates.into_iter().flatten() {
        match update {
            ReplicaUpdateV3::Snapshot(publication) => {
                assert_eq!(replica.apply_snapshot(&publication), Ok(()));
            }
            ReplicaUpdateV3::Delta(frame) => {
                assert_eq!(replica.apply_delta(&frame), Ok(()));
            }
        }
    }

    assert_eq!(
        replica.entities(),
        &[row(1, hf(10, 10, false), FactSet::default())]
    );
    assert_eq!(
        replica.turn(),
        Some(&TurnFact {
            active: Some(id(2)),
            round: 3,
        })
    );
    assert_eq!(replica.encounter(), &bx.encounter);
    assert_eq!(replica.timeline(), Some(&[tl(1, 1)][..]));
    assert_eq!(
        replica.pause(),
        Some(PauseState::Paused {
            reason: Some(PauseReason::Player)
        })
    );
    assert_eq!(replica.host_facts(), None);
    assert_eq!(replica.next_event_seq(), 5);
    assert_eq!(replica.server_tick(), 52);
    refused(
        &mut replica,
        &f(5, vec![enter(2)]),
        ReplicaError::ReusedEntityId { entity: id(2) },
    );
}
