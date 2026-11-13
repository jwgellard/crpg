//! T140r2 two-keys suite: the v3 replica (`replica_v3`), written from
//! `tasks/T140r.md` (Behaviour R-1..R-15, Tests R1..R10) and the public API
//! only. The implementer's suite and source were not read.
//!
//! Expected values are contract literals or hand arithmetic, never output of
//! the code under test. Every refusal has an exact error, a replica compared
//! whole against its pre-call clone, and a passing control that differs in
//! one input.

use crpg_core::Ulid;
use crpg_net::codec_v3;
use crpg_net::pool_v3::{
    DeltaOutcome, GateMode, PublicationV3, ReassemblyPoolV3, ReplicaUpdateV3, TransferLimitsV3,
    TransferOutcome,
};
use crpg_net::protocol::{
    DeltaOp as OpV1, ReceiptStatus as ReceiptV1, MAX_DELTA_OPS, MAX_VISIBLE_ENTITIES, SEQ_LAST,
};
use crpg_net::protocol_transfer::{HealthFact, SnapshotBody, SnapshotEntity, TransferId, TurnFact};
use crpg_net::protocol_v2::{DeltaFrame as FrameV2, DeltaOp as OpV2};
use crpg_net::protocol_v3::{
    AutoPauseToggles, AutoPauseTrigger, ClockFact, DeltaFrame, DeltaOp, EncounterFacts,
    EncounterMode, FactSet, HostFacts, NetId, OrderFact, PauseReason, PauseRights, PoolFact,
    ReceiptStatus, SessionEpoch, StatFact, StatValue, TimelineEntry,
};
use crpg_net::reassembly::Publication;
use crpg_net::replica::ReplicaWorld;
use crpg_net::replica_v3::{
    PauseState, ReplicaEntityV3, ReplicaError, ReplicaWorldV3, MAX_REPLICA_ENTITIES,
};
use crpg_net::transfer_v3::{self as t3, SnapshotBodyV3, SnapshotEntityV3};

const Z: [u8; 16] = [0; 16];

// ---------------------------------------------------------------------------
// Fixtures.
// ---------------------------------------------------------------------------

fn id(n: u64) -> NetId {
    NetId::new(n).expect("nonzero net id")
}

fn tid(n: u64) -> TransferId {
    TransferId::new(n).expect("nonzero transfer id")
}

fn u(n: u128) -> Ulid {
    Ulid::from_u128(n)
}

fn rep() -> ReplicaWorldV3 {
    ReplicaWorldV3::new(SessionEpoch(Z))
}

fn hf(health: u32, max_health: u32, dead: bool) -> HealthFact {
    HealthFact {
        health,
        max_health,
        dead,
    }
}

fn turn(active: Option<u64>, round: u64) -> TurnFact {
    TurnFact {
        active: active.map(id),
        round,
    }
}

/// `F(n, ops)`: lane 0, tick `n`, `event_seq` `n`.
fn fr(n: u64, ops: Vec<DeltaOp>) -> DeltaFrame {
    DeltaFrame {
        lane: 0,
        server_tick: n,
        event_seq: n,
        ops,
    }
}

/// `F(n, ops)` with an explicit tick.
fn frt(n: u64, tick: u64, ops: Vec<DeltaOp>) -> DeltaFrame {
    DeltaFrame {
        lane: 0,
        server_tick: tick,
        event_seq: n,
        ops,
    }
}

fn enter(n: u64) -> DeltaOp {
    DeltaOp::EntityEnter { entity: id(n) }
}

fn leave(n: u64) -> DeltaOp {
    DeltaOp::EntityLeave { entity: id(n) }
}

fn health_op(n: u64, health: u32, max_health: u32, dead: bool) -> DeltaOp {
    DeltaOp::Health {
        entity: id(n),
        health,
        max_health,
        dead,
    }
}

fn turn_op(active: Option<u64>, round: u64) -> DeltaOp {
    DeltaOp::Turn {
        active: active.map(id),
        round,
    }
}

fn facts_op(n: u64, facts: FactSet) -> DeltaOp {
    DeltaOp::EntityFacts {
        entity: id(n),
        facts,
    }
}

fn timeline_op(entries: Option<Vec<TimelineEntry>>) -> DeltaOp {
    DeltaOp::Timeline { entries }
}

fn tl(key: i32, n: u64) -> TimelineEntry {
    TimelineEntry { key, entity: id(n) }
}

fn ord(t: u64) -> Option<Option<OrderFact>> {
    Some(Some(OrderFact {
        ability: u(2),
        target: id(t),
        repeat: true,
    }))
}

/// `FS(order: ord(t))`.
fn fs_order(t: u64) -> FactSet {
    FactSet {
        order: ord(t),
        ..FactSet::default()
    }
}

fn fs_a() -> FactSet {
    FactSet {
        pools: Some(vec![PoolFact {
            pool: u(1),
            current: 3,
            max: 5,
        }]),
        effects: Some(vec![]),
        order: ord(2),
        stats: None,
        tags: Some(vec!["elite".to_string()]),
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

fn host_h() -> HostFacts {
    HostFacts {
        rights: PauseRights {
            pause: true,
            unpause: false,
        },
        auto_pause: Some(AutoPauseToggles::from_bits(0x05).expect("bits 0 and 2")),
        turn_timer_ms: Some(None),
    }
}

fn row(n: u64) -> SnapshotEntityV3 {
    SnapshotEntityV3 {
        entity: id(n),
        health: None,
        facts: FactSet::default(),
    }
}

fn rowf(n: u64, facts: FactSet) -> SnapshotEntityV3 {
    SnapshotEntityV3 {
        entity: id(n),
        health: None,
        facts,
    }
}

fn rows(ids: &[u64]) -> Vec<SnapshotEntityV3> {
    ids.iter().copied().map(row).collect()
}

fn body(
    entities: Vec<SnapshotEntityV3>,
    turn: Option<TurnFact>,
    timeline: Option<Vec<TimelineEntry>>,
) -> SnapshotBodyV3 {
    SnapshotBodyV3 {
        entities,
        turn,
        encounter: EncounterFacts::default(),
        timeline,
    }
}

/// `E0`: the empty v3 body.
fn e0() -> SnapshotBodyV3 {
    body(vec![], None, None)
}

/// `P(cursor, base, body)`.
fn p(cursor: u64, base: u64, body: SnapshotBodyV3) -> PublicationV3 {
    PublicationV3 {
        transfer: tid(1),
        base_tick: base,
        cursor,
        body,
    }
}

fn ent(n: u64, health: Option<HealthFact>, facts: FactSet) -> ReplicaEntityV3 {
    ReplicaEntityV3 {
        net_id: id(n),
        health,
        facts,
    }
}

fn dflt(n: u64) -> ReplicaEntityV3 {
    ent(n, None, FactSet::default())
}

fn facts_of(r: &ReplicaWorldV3, n: u64) -> FactSet {
    r.entity(id(n)).expect("row is live").facts.clone()
}

fn health_of(r: &ReplicaWorldV3, n: u64) -> Option<Option<HealthFact>> {
    r.entity(id(n)).map(|e| e.health.clone())
}

fn apply_ok(r: &mut ReplicaWorldV3, frame: DeltaFrame) {
    assert_eq!(r.apply_delta(&frame), Ok(()), "frame {frame:?}");
}

fn snap_ok(r: &mut ReplicaWorldV3, publication: PublicationV3) {
    assert_eq!(r.apply_snapshot(&publication), Ok(()));
}

/// The frame is refused with `err` and the replica equals its clone.
fn refuse(r: &mut ReplicaWorldV3, frame: DeltaFrame, err: ReplicaError) {
    let before = r.clone();
    assert_eq!(r.apply_delta(&frame), Err(err), "frame {frame:?}");
    assert_eq!(*r, before, "a refused frame changed the replica");
}

/// The publication is refused with `err` and the replica equals its clone.
fn refuse_snap(r: &mut ReplicaWorldV3, publication: PublicationV3, err: ReplicaError) {
    let before = r.clone();
    assert_eq!(r.apply_snapshot(&publication), Err(err));
    assert_eq!(*r, before, "a refused publication changed the replica");
}

/// A passing control: the publication applies to a clone.
fn control_snap(r: &ReplicaWorldV3, publication: PublicationV3) -> ReplicaWorldV3 {
    let mut c = r.clone();
    snap_ok(&mut c, publication);
    c
}

fn unknown(n: u64) -> ReplicaError {
    ReplicaError::UnknownEntity { entity: id(n) }
}

fn tl_vec(r: &ReplicaWorldV3) -> Option<Vec<TimelineEntry>> {
    r.timeline().map(<[TimelineEntry]>::to_vec)
}

// ---------------------------------------------------------------------------
// R1.
// ---------------------------------------------------------------------------

/// R-1, R-12 (fresh queries), R-5 (epoch of a fresh replica).
#[test]
fn r1_construction() {
    let r = rep();
    assert_eq!(r.epoch(), SessionEpoch(Z));
    assert_eq!(r.entities(), &[] as &[ReplicaEntityV3]);
    assert_eq!(r.len(), 0);
    assert!(r.is_empty());
    assert_eq!(r.entity(id(1)), None);
    assert_eq!(r.turn(), None);
    assert_eq!(*r.encounter(), EncounterFacts::default());
    assert_eq!(r.timeline(), None);
    assert_eq!(r.pause(), None);
    assert_eq!(r.host_facts(), None);
    assert_eq!(r.next_event_seq(), 1);
    assert_eq!(r.server_tick(), 0);
    // High-water 0: id 1 is a legal first enter.
    let mut c = r.clone();
    apply_ok(&mut c, fr(1, vec![enter(1)]));
    assert_eq!(c.entities(), &[dflt(1)]);
    // Two fresh replicas of one epoch are equal (R-13).
    assert_eq!(rep(), rep());
    // The epoch is the argument, and a receipt is checked against it.
    let other = SessionEpoch([7; 16]);
    let mut o = ReplicaWorldV3::new(other);
    assert_eq!(o.epoch(), other);
    let receipt = |epoch: [u8; 16]| DeltaOp::Receipt {
        epoch,
        lane: 0,
        seq: 1,
        processed_tick: 0,
        status: ReceiptStatus::Applied,
    };
    refuse(&mut o, fr(1, vec![receipt(Z)]), ReplicaError::WrongEpoch);
    apply_ok(&mut o, fr(1, vec![receipt([7; 16])]));
    assert_eq!(o.epoch(), other);
}

// ---------------------------------------------------------------------------
// R2: differential against the v2 replica.
// ---------------------------------------------------------------------------

/// A v2 replica and a v3 replica driven with the same script.
struct Duo {
    v2: ReplicaWorld,
    v3: ReplicaWorldV3,
}

fn l(op: OpV1) -> OpV2 {
    OpV2::Legacy(op)
}

fn e2(n: u64) -> OpV2 {
    l(OpV1::EntityEnter { entity: id(n) })
}

fn lv2(n: u64) -> OpV2 {
    l(OpV1::EntityLeave { entity: id(n) })
}

fn health2(n: u64, h: u32, m: u32, dead: bool) -> OpV2 {
    l(OpV1::Health {
        entity: id(n),
        health: h,
        max_health: m,
        dead,
    })
}

fn turn2(active: Option<u64>, round: u64) -> OpV2 {
    l(OpV1::Turn {
        active: active.map(id),
        round,
    })
}

fn resolved2(actor: u64, target: u64) -> OpV2 {
    OpV2::ActionResolved {
        actor: id(actor),
        target: id(target),
        ability: u(1),
        outcome: "success".to_string(),
        damage: 0,
    }
}

impl Duo {
    fn new() -> Self {
        Self {
            v2: ReplicaWorld::new(SessionEpoch(Z)),
            v3: rep(),
        }
    }

    fn delta(
        &mut self,
        seq: u64,
        tick: u64,
        lane: u8,
        ops: Vec<OpV2>,
        expect: Result<(), ReplicaError>,
    ) {
        let f2 = FrameV2 {
            lane,
            server_tick: tick,
            event_seq: seq,
            ops,
        };
        let f3 = DeltaFrame {
            lane,
            server_tick: tick,
            event_seq: seq,
            ops: f2.ops.iter().cloned().map(DeltaOp::from).collect(),
        };
        let (b2, b3) = (self.v2.clone(), self.v3.clone());
        assert_eq!(self.v2.apply_delta(&f2), expect, "v2 replica, seq {seq}");
        assert_eq!(self.v3.apply_delta(&f3), expect, "v3 replica, seq {seq}");
        if expect.is_err() {
            assert_eq!(self.v2, b2);
            assert_eq!(self.v3, b3);
        }
        self.agree();
    }

    fn ok(&mut self, seq: u64, ops: Vec<OpV2>) {
        self.delta(seq, seq, 0, ops, Ok(()));
    }

    fn err(&mut self, seq: u64, ops: Vec<OpV2>, e: ReplicaError) {
        self.delta(seq, seq, 0, ops, Err(e));
    }

    fn snap(
        &mut self,
        cursor: u64,
        base: u64,
        rows3: &[(u64, Option<HealthFact>)],
        turn: Option<TurnFact>,
        expect: Result<(), ReplicaError>,
    ) {
        let p2 = Publication {
            transfer: tid(1),
            base_tick: base,
            cursor,
            body: SnapshotBody {
                entities: rows3
                    .iter()
                    .map(|(n, h)| SnapshotEntity {
                        entity: id(*n),
                        health: h.clone(),
                    })
                    .collect(),
                turn: turn.clone(),
            },
        };
        let p3 = PublicationV3 {
            transfer: tid(1),
            base_tick: base,
            cursor,
            body: body(
                rows3
                    .iter()
                    .map(|(n, h)| SnapshotEntityV3 {
                        entity: id(*n),
                        health: h.clone(),
                        facts: FactSet::default(),
                    })
                    .collect(),
                turn,
                None,
            ),
        };
        let (b2, b3) = (self.v2.clone(), self.v3.clone());
        assert_eq!(self.v2.apply_snapshot(&p2), expect);
        assert_eq!(self.v3.apply_snapshot(&p3), expect);
        if expect.is_err() {
            assert_eq!(self.v2, b2);
            assert_eq!(self.v3, b3);
        }
        self.agree();
    }

    fn agree(&self) {
        let r2: Vec<(NetId, Option<HealthFact>)> = self
            .v2
            .entities()
            .iter()
            .map(|e| (e.net_id, e.health.clone()))
            .collect();
        let r3: Vec<(NetId, Option<HealthFact>)> = self
            .v3
            .entities()
            .iter()
            .map(|e| (e.net_id, e.health.clone()))
            .collect();
        assert_eq!(r2, r3);
        assert!(self
            .v3
            .entities()
            .iter()
            .all(|e| e.facts == FactSet::default()));
        assert!(r3.windows(2).all(|w| w[0].0 < w[1].0), "rows ascending");
        assert_eq!(self.v2.turn(), self.v3.turn());
        assert_eq!(self.v2.next_event_seq(), self.v3.next_event_seq());
        assert_eq!(self.v2.server_tick(), self.v3.server_tick());
        assert_eq!(self.v2.len(), self.v3.len());
        // No v3 state appears from v2 input.
        assert_eq!(*self.v3.encounter(), EncounterFacts::default());
        assert_eq!(self.v3.timeline(), None);
        assert_eq!(self.v3.pause(), None);
        assert_eq!(self.v3.host_facts(), None);
    }
}

/// R-2, R-4, R-5, R-13, R-14: the 25-step script of R2.
#[test]
fn r2_matches_the_v2_replica() {
    let mut d = Duo::new();

    // 1.
    d.delta(
        1,
        10,
        0,
        vec![
            e2(1),
            e2(2),
            health2(1, 3, 10, false),
            turn2(Some(1), 1),
            resolved2(1, 2),
            OpV2::TurnStarted {
                actor: id(1),
                round: 1,
            },
            l(OpV1::Receipt {
                epoch: Z,
                lane: 0,
                seq: 1,
                processed_tick: 0,
                status: ReceiptV1::Applied,
            }),
            l(OpV1::Spawned { entity: id(2) }),
            l(OpV1::Died { entity: id(2) }),
        ],
        Ok(()),
    );
    assert_eq!(
        d.v3.entities(),
        &[ent(1, Some(hf(3, 10, false)), FactSet::default()), dflt(2)]
    );
    assert_eq!(d.v3.turn(), Some(&turn(Some(1), 1)));
    assert_eq!((d.v3.next_event_seq(), d.v3.server_tick()), (2, 10));

    // 2. Wrong sequence.
    d.delta(
        1,
        11,
        0,
        vec![],
        Err(ReplicaError::EventSeq {
            expected: 2,
            got: 1,
        }),
    );
    // 3. Wrong lane.
    d.delta(2, 2, 1, vec![], Err(ReplicaError::WrongLane { got: 1 }));
    // 4. 257 ops refused (256 pass in the boundary test below).
    d.err(
        2,
        (0..257)
            .map(|_| l(OpV1::Spawned { entity: id(1) }))
            .collect(),
        ReplicaError::TooManyOps { got: 257 },
    );
    assert_eq!(MAX_DELTA_OPS, 256);
    // 5.
    d.err(2, vec![health2(9, 1, 1, false)], unknown(9));
    // 6.
    d.err(
        2,
        vec![e2(1)],
        ReplicaError::DuplicateEntity { entity: id(1) },
    );
    // 7.
    d.delta(2, 12, 0, vec![lv2(2)], Ok(()));
    assert_eq!(d.v3.entity(id(2)), None);
    // 8.
    d.err(
        3,
        vec![e2(2)],
        ReplicaError::ReusedEntityId { entity: id(2) },
    );
    // 9.
    d.err(
        3,
        vec![l(OpV1::Receipt {
            epoch: [1; 16],
            lane: 0,
            seq: 1,
            processed_tick: 0,
            status: ReceiptV1::Applied,
        })],
        ReplicaError::WrongEpoch,
    );
    // 10.
    d.delta(
        3,
        13,
        0,
        vec![OpV2::EncounterEnded {
            encounter: u(3),
            round: 1,
        }],
        Ok(()),
    );
    // EncounterEnded clears neither the turn nor anything else.
    assert_eq!(d.v3.turn(), Some(&turn(Some(1), 1)));

    // 11.
    d.snap(
        3,
        20,
        &[(1, Some(hf(5, 10, false))), (3, None)],
        Some(turn(Some(3), 2)),
        Ok(()),
    );
    assert_eq!(
        d.v3.entities(),
        &[ent(1, Some(hf(5, 10, false)), FactSet::default()), dflt(3)]
    );
    assert_eq!(d.v3.turn(), Some(&turn(Some(3), 2)));
    assert_eq!((d.v3.next_event_seq(), d.v3.server_tick()), (4, 20));
    // 12. High-water is 3 after the snapshot.
    d.err(
        4,
        vec![e2(2)],
        ReplicaError::ReusedEntityId { entity: id(2) },
    );
    // 13.
    d.delta(4, 21, 0, vec![e2(4), health2(4, 1, 1, true)], Ok(()));
    assert_eq!(
        d.v3.entity(id(4)),
        Some(&ent(4, Some(hf(1, 1, true)), FactSet::default()))
    );
    // 14.
    d.snap(
        2,
        0,
        &[],
        None,
        Err(ReplicaError::SnapshotCursor { min: 4, got: 2 }),
    );
    // 15.
    d.err(5, vec![l(OpV1::Spawned { entity: id(9) })], unknown(9));
    d.err(5, vec![l(OpV1::Died { entity: id(9) })], unknown(9));
    d.err(5, vec![lv2(9)], unknown(9));
    d.err(5, vec![l(OpV1::Despawned { entity: id(9) })], unknown(9));
    // 16.
    d.err(5, vec![turn2(Some(9), 3)], unknown(9));
    // 17. Actor, then target.
    d.err(5, vec![resolved2(9, 8)], unknown(9));
    d.err(5, vec![resolved2(1, 8)], unknown(8));
    // 18.
    d.err(
        5,
        vec![OpV2::TurnStarted {
            actor: id(9),
            round: 3,
        }],
        unknown(9),
    );
    // 19.
    d.delta(5, 22, 0, vec![l(OpV1::Despawned { entity: id(4) })], Ok(()));
    assert_eq!(d.v3.entity(id(4)), None);
    // 20.
    d.err(6, vec![l(OpV1::Despawned { entity: id(4) })], unknown(4));

    // 21. Fill to 2,048 rows.
    for seq in 6..=12u64 {
        let first = (seq - 6) * 256 + 5;
        d.ok(seq, (first..first + 256).map(e2).collect());
    }
    d.ok(13, (1797..=2050).map(e2).collect());
    assert_eq!(d.v3.len(), 2_048);
    assert_eq!(d.v2.len(), 2_048);
    assert_eq!(MAX_REPLICA_ENTITIES, 2_048);
    assert_eq!(d.v3.entity(id(2050)), Some(&dflt(2050)));
    assert_eq!(d.v3.entity(id(4)), None);
    assert_eq!(d.v3.entity(id(2051)), None);
    // 22. One past the bound.
    d.err(14, vec![e2(2051)], ReplicaError::TooManyEntities);
    // 23. A leave first frees the slot.
    d.ok(14, vec![lv2(1), e2(2051)]);
    assert_eq!(d.v3.len(), 2_048);
    assert_eq!(d.v3.entity(id(1)), None);
    assert_eq!(d.v3.entity(id(2051)), Some(&dflt(2051)));

    // 24.
    d.snap(SEQ_LAST, 30, &[], None, Ok(()));
    assert_eq!(d.v3.next_event_seq(), u64::MAX);
    assert_eq!(d.v3.server_tick(), 30);
    assert!(d.v3.is_empty());
    // 25. Nothing is accepted any more.
    d.err(
        u64::MAX,
        vec![],
        ReplicaError::EventSeq {
            expected: u64::MAX,
            got: u64::MAX,
        },
    );
}

/// R-2, R-3, R-4: boundaries, precedence of the checks, the working-state
/// rule and the sequence law.
#[test]
fn r2_precedence_boundaries_and_working_state() {
    let mut r = rep();
    apply_ok(&mut r, fr(1, vec![enter(1)]));
    let spawns =
        |n: usize| -> Vec<DeltaOp> { (0..n).map(|_| DeltaOp::Spawned { entity: id(1) }).collect() };
    // 256 ops pass; 257 do not.
    let mut c = r.clone();
    apply_ok(&mut c, fr(2, spawns(256)));
    assert_eq!(c.next_event_seq(), 3);
    assert_eq!(c.entities(), r.entities());
    // (a) beats (b) beats (c) beats (d).
    let mut bad_lane = fr(9, spawns(257));
    bad_lane.lane = 1;
    refuse(&mut r, bad_lane, ReplicaError::WrongLane { got: 1 });
    refuse(
        &mut r,
        fr(9, spawns(257)),
        ReplicaError::TooManyOps { got: 257 },
    );
    refuse(
        &mut r,
        fr(9, vec![health_op(8, 1, 1, false)]),
        ReplicaError::EventSeq {
            expected: 2,
            got: 9,
        },
    );
    refuse(
        &mut r,
        fr(1, vec![]),
        ReplicaError::EventSeq {
            expected: 2,
            got: 1,
        },
    );
    refuse(
        &mut r,
        fr(0, vec![]),
        ReplicaError::EventSeq {
            expected: 2,
            got: 0,
        },
    );
    // First faulty op wins, in frame order.
    refuse(
        &mut r,
        fr(2, vec![health_op(9, 1, 1, false), enter(1)]),
        unknown(9),
    );
    refuse(
        &mut r,
        fr(2, vec![enter(1), health_op(9, 1, 1, false)]),
        ReplicaError::DuplicateEntity { entity: id(1) },
    );
    // Later ops see what earlier ops left.
    let mut c = r.clone();
    apply_ok(
        &mut c,
        fr(
            2,
            vec![enter(3), health_op(3, 4, 5, false), leave(3), enter(4)],
        ),
    );
    assert_eq!(c.entities(), &[dflt(1), dflt(4)]);
    refuse(
        &mut r,
        fr(2, vec![leave(1), health_op(1, 1, 1, false)]),
        unknown(1),
    );
    refuse(
        &mut r,
        fr(2, vec![enter(5), leave(5), enter(5)]),
        ReplicaError::ReusedEntityId { entity: id(5) },
    );
    // R-4: the sequence law holds for no ops at all, and the tick is the
    // frame's even when lower.
    apply_ok(&mut r, frt(2, 77, vec![]));
    assert_eq!((r.next_event_seq(), r.server_tick()), (3, 77));
    apply_ok(&mut r, frt(3, 5, vec![]));
    assert_eq!((r.next_event_seq(), r.server_tick()), (4, 5));
    // The last legal sequence number, then nothing.
    snap_ok(&mut r, p(SEQ_LAST - 1, 6, e0()));
    assert_eq!(r.next_event_seq(), SEQ_LAST);
    apply_ok(&mut r, frt(SEQ_LAST, 8, vec![]));
    assert_eq!((r.next_event_seq(), r.server_tick()), (u64::MAX, 8));
    refuse(
        &mut r,
        frt(u64::MAX, 9, vec![]),
        ReplicaError::EventSeq {
            expected: u64::MAX,
            got: u64::MAX,
        },
    );
}

/// R-5: Health and Turn replace whole; Turn(None) is legal; a removal takes
/// the row; notices of tags 2..=4 and 10 change nothing.
#[test]
fn r2_health_and_turn_replace_whole() {
    let mut r = rep();
    apply_ok(
        &mut r,
        fr(1, vec![enter(1), enter(2), health_op(1, 3, 10, false)]),
    );
    assert_eq!(health_of(&r, 1), Some(Some(hf(3, 10, false))));
    // Stored verbatim: zero health with the dead flag.
    apply_ok(&mut r, fr(2, vec![health_op(1, 0, 10, true)]));
    assert_eq!(health_of(&r, 1), Some(Some(hf(0, 10, true))));
    assert_eq!(health_of(&r, 2), Some(None));
    apply_ok(&mut r, fr(3, vec![turn_op(Some(2), 4)]));
    assert_eq!(r.turn(), Some(&turn(Some(2), 4)));
    apply_ok(&mut r, fr(4, vec![turn_op(None, 5)]));
    assert_eq!(r.turn(), Some(&turn(None, 5)));
    let before = r.entities().to_vec();
    apply_ok(
        &mut r,
        fr(
            5,
            vec![
                DeltaOp::Spawned { entity: id(2) },
                DeltaOp::Died { entity: id(2) },
                DeltaOp::EncounterEnded {
                    encounter: u(1),
                    round: 9,
                },
            ],
        ),
    );
    assert_eq!(r.entities(), before.as_slice());
    assert_eq!(r.turn(), Some(&turn(None, 5)));
    // Despawned removes the row with its health.
    apply_ok(&mut r, fr(6, vec![DeltaOp::Despawned { entity: id(1) }]));
    assert_eq!(r.entities(), &[dflt(2)]);
}

// ---------------------------------------------------------------------------
// R3.
// ---------------------------------------------------------------------------

/// R-6, R-12: the fact ops replace their images whole.
#[test]
fn r3_fact_ops_replace_images() {
    let mut r = rep();
    apply_ok(&mut r, fr(1, vec![enter(1), enter(2)]));
    assert_eq!(r.entities(), &[dflt(1), dflt(2)]);
    // F(2): row 1 gets FS_A, row 2 is untouched.
    apply_ok(&mut r, fr(2, vec![facts_op(1, fs_a())]));
    assert_eq!(r.entities(), &[ent(1, None, fs_a()), dflt(2)]);
    // F(3): the new image replaces; pools, effects and tags are gone.
    let only_order = FactSet {
        order: Some(None),
        ..FactSet::default()
    };
    apply_ok(&mut r, fr(3, vec![facts_op(1, only_order.clone())]));
    assert_eq!(facts_of(&r, 1), only_order);
    assert_eq!(facts_of(&r, 1).pools, None);
    assert_eq!(facts_of(&r, 1).effects, None);
    assert_eq!(facts_of(&r, 1).tags, None);
    assert_eq!(facts_of(&r, 2), FactSet::default());
    // F(4): all withheld is "forget".
    apply_ok(&mut r, fr(4, vec![facts_op(1, FactSet::default())]));
    assert_eq!(facts_of(&r, 1), FactSet::default());
    // F(5)..F(7): the encounter image.
    apply_ok(&mut r, fr(5, vec![DeltaOp::Encounter(ef_a())]));
    assert_eq!(*r.encounter(), ef_a());
    let tb = EncounterFacts {
        mode: Some(EncounterMode::TurnBased),
        ..EncounterFacts::default()
    };
    apply_ok(&mut r, fr(6, vec![DeltaOp::Encounter(tb)]));
    assert_eq!(*r.encounter(), tb);
    assert_eq!(r.encounter().clock, None);
    assert_eq!(r.encounter().pending_mode, None);
    apply_ok(
        &mut r,
        fr(7, vec![DeltaOp::Encounter(EncounterFacts::default())]),
    );
    assert_eq!(*r.encounter(), EncounterFacts::default());
    // F(8)..F(10): the timeline image, input order kept.
    apply_ok(
        &mut r,
        fr(8, vec![timeline_op(Some(vec![tl(3, 2), tl(3, 1)]))]),
    );
    assert_eq!(r.timeline(), Some(&[tl(3, 2), tl(3, 1)][..]));
    apply_ok(&mut r, fr(9, vec![timeline_op(Some(vec![]))]));
    assert_eq!(r.timeline(), Some(&[][..]));
    apply_ok(&mut r, fr(10, vec![timeline_op(None)]));
    assert_eq!(r.timeline(), None);
    // F(11)..F(13): a removal takes the image; a new row starts clean.
    apply_ok(&mut r, fr(11, vec![facts_op(2, fs_order(1))]));
    assert_eq!(r.entities(), &[dflt(1), ent(2, None, fs_order(1))]);
    apply_ok(&mut r, fr(12, vec![leave(2)]));
    assert_eq!(r.entity(id(2)), None);
    apply_ok(&mut r, fr(13, vec![enter(3)]));
    assert_eq!(r.entities(), &[dflt(1), dflt(3)]);
    assert_eq!(facts_of(&r, 3), FactSet::default());
    assert_eq!(r.next_event_seq(), 14);
    // Encounter and an empty timeline need no live row.
    let mut fresh = rep();
    apply_ok(
        &mut fresh,
        fr(
            1,
            vec![DeltaOp::Encounter(ef_a()), timeline_op(Some(vec![]))],
        ),
    );
    assert_eq!(*fresh.encounter(), ef_a());
    assert_eq!(fresh.timeline(), Some(&[][..]));
    assert!(fresh.is_empty());
}

// ---------------------------------------------------------------------------
// R4.
// ---------------------------------------------------------------------------

/// The accessors R4 compares for "unchanged".
fn assert_unchanged(after: &ReplicaWorldV3, before: &ReplicaWorldV3) {
    assert_eq!(after.entities(), before.entities());
    assert_eq!(after.turn(), before.turn());
    assert_eq!(after.encounter(), before.encounter());
    assert_eq!(after.timeline(), before.timeline());
    assert_eq!(after.pause(), before.pause());
    assert_eq!(after.host_facts(), before.host_facts());
}

/// R-6, R-7, R-3, R-2: dangling fact ids refused in order, notices inert.
#[test]
fn r4_fact_references_live_and_notices_inert() {
    let mut r = rep();
    apply_ok(&mut r, fr(1, vec![enter(1), enter(2)]));

    refuse(
        &mut r,
        fr(2, vec![facts_op(9, FactSet::default())]),
        unknown(9),
    );
    refuse(&mut r, fr(2, vec![facts_op(1, fs_order(9))]), unknown(9));
    // Entity before target.
    refuse(&mut r, fr(2, vec![facts_op(8, fs_order(9))]), unknown(8));
    refuse(
        &mut r,
        fr(
            2,
            vec![timeline_op(Some(vec![tl(0, 1), tl(1, 9), tl(2, 8)]))],
        ),
        unknown(9),
    );
    refuse(
        &mut r,
        fr(2, vec![facts_op(4, FactSet::default()), enter(4)]),
        unknown(4),
    );
    // Controls that differ in one input.
    let mut c = r.clone();
    apply_ok(&mut c, fr(2, vec![facts_op(1, fs_order(2))]));
    let mut c = r.clone();
    apply_ok(
        &mut c,
        fr(2, vec![timeline_op(Some(vec![tl(0, 1), tl(1, 2)]))]),
    );
    let mut c = r.clone();
    apply_ok(
        &mut c,
        fr(2, vec![enter(4), facts_op(4, FactSet::default())]),
    );

    // 1.
    apply_ok(
        &mut r,
        fr(
            2,
            vec![
                enter(3),
                facts_op(3, fs_order(3)),
                timeline_op(Some(vec![tl(0, 3)])),
                turn_op(Some(3), 4),
                DeltaOp::Encounter(ef_a()),
            ],
        ),
    );
    // 2. Notices naming live and dead ids alike.
    let before = r.clone();
    apply_ok(
        &mut r,
        fr(
            3,
            vec![
                DeltaOp::OrderEnded { actor: id(3) },
                DeltaOp::Joined { entity: id(3) },
                DeltaOp::Left { entity: id(3) },
                DeltaOp::OrderEnded { actor: id(9) },
                DeltaOp::Joined { entity: id(9) },
                DeltaOp::Left { entity: id(9) },
            ],
        ),
    );
    assert_unchanged(&r, &before);
    assert_eq!(facts_of(&r, 3), fs_order(3));
    assert_eq!(r.len(), 3);
    assert_eq!(r.next_event_seq(), 4);
    // 3.
    let before = r.clone();
    apply_ok(
        &mut r,
        fr(
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
            ],
        ),
    );
    assert_unchanged(&r, &before);
    assert_eq!(r.turn(), Some(&turn(Some(3), 4)));
    assert_eq!(*r.encounter(), ef_a());
    assert_eq!(r.encounter().mode, Some(EncounterMode::RealTime));
    assert_eq!(r.timeline(), Some(&[tl(0, 3)][..]));
    assert_eq!(r.next_event_seq(), 5);
    // 4. Descending keys and a repeated entity are stored as given.
    apply_ok(
        &mut r,
        fr(5, vec![timeline_op(Some(vec![tl(5, 1), tl(4, 1)]))]),
    );
    assert_eq!(r.timeline(), Some(&[tl(5, 1), tl(4, 1)][..]));
    // 5. A pool with current > max is stored as given.
    let odd = FactSet {
        pools: Some(vec![PoolFact {
            pool: u(1),
            current: 9,
            max: 3,
        }]),
        ..FactSet::default()
    };
    apply_ok(&mut r, fr(6, vec![facts_op(1, odd.clone())]));
    assert_eq!(facts_of(&r, 1), odd);
    assert_eq!(r.next_event_seq(), 7);
}

// ---------------------------------------------------------------------------
// R5.
// ---------------------------------------------------------------------------

/// R-9: stored references go stale without being touched; new ones are
/// checked.
#[test]
fn r5_stale_references_are_tolerated() {
    let mut r = rep();
    apply_ok(
        &mut r,
        fr(
            1,
            vec![
                enter(1),
                enter(2),
                facts_op(1, fs_order(2)),
                timeline_op(Some(vec![tl(0, 1), tl(1, 2)])),
                turn_op(Some(2), 1),
            ],
        ),
    );
    apply_ok(&mut r, fr(2, vec![leave(2)]));
    assert_eq!(r.entity(id(2)), None);
    assert_eq!(facts_of(&r, 1), fs_order(2));
    assert_eq!(tl_vec(&r), Some(vec![tl(0, 1), tl(1, 2)]));
    assert_eq!(r.turn(), Some(&turn(Some(2), 1)));
    // Despawned removes the same way; the references of the others stay.
    let mut c = r.clone();
    apply_ok(
        &mut c,
        fr(3, vec![enter(3), DeltaOp::Despawned { entity: id(1) }]),
    );
    assert_eq!(c.timeline(), r.timeline());
    assert_eq!(c.turn(), r.turn());
    // A new op naming the removed id is refused.
    refuse(&mut r, fr(3, vec![facts_op(1, fs_order(2))]), unknown(2));
    refuse(
        &mut r,
        fr(3, vec![timeline_op(Some(vec![tl(0, 2)]))]),
        unknown(2),
    );
    refuse(&mut r, fr(3, vec![turn_op(Some(2), 2)]), unknown(2));
    // Control: the same op naming a live id.
    apply_ok(&mut r, fr(3, vec![facts_op(1, fs_order(1))]));
}

// ---------------------------------------------------------------------------
// R6.
// ---------------------------------------------------------------------------

/// R-8, R-11: pause and host facts, and a snapshot leaves both alone.
#[test]
fn r6_clock_and_host_facts() {
    let mut r = rep();
    apply_ok(&mut r, fr(1, vec![DeltaOp::Clock { paused: None }]));
    assert_eq!(r.pause(), Some(PauseState::Running));
    apply_ok(&mut r, fr(2, vec![DeltaOp::Clock { paused: Some(None) }]));
    assert_eq!(r.pause(), Some(PauseState::Paused { reason: None }));
    let died = PauseReason::Auto(AutoPauseTrigger::PartyMemberDied);
    apply_ok(
        &mut r,
        fr(
            3,
            vec![DeltaOp::Clock {
                paused: Some(Some(died)),
            }],
        ),
    );
    assert_eq!(r.pause(), Some(PauseState::Paused { reason: Some(died) }));
    assert_eq!(r.host_facts(), None);
    apply_ok(&mut r, fr(4, vec![DeltaOp::HostFacts(host_h())]));
    assert_eq!(r.host_facts(), Some(&host_h()));
    snap_ok(&mut r, p(4, 9, e0()));
    assert_eq!(r.pause(), Some(PauseState::Paused { reason: Some(died) }));
    assert_eq!(r.host_facts(), Some(&host_h()));
    assert_eq!((r.next_event_seq(), r.server_tick()), (5, 9));
    apply_ok(&mut r, fr(5, vec![DeltaOp::Clock { paused: None }]));
    assert_eq!(r.pause(), Some(PauseState::Running));
    assert_eq!(r.host_facts(), Some(&host_h()));
    // The other reasons map to themselves; a later HostFacts replaces.
    for (seq, reason) in [
        (6, PauseReason::Player),
        (7, PauseReason::Operator),
        (8, PauseReason::Restore),
    ] {
        apply_ok(
            &mut r,
            fr(
                seq,
                vec![DeltaOp::Clock {
                    paused: Some(Some(reason)),
                }],
            ),
        );
        assert_eq!(
            r.pause(),
            Some(PauseState::Paused {
                reason: Some(reason)
            })
        );
    }
    let h2 = HostFacts {
        rights: PauseRights::default(),
        auto_pause: None,
        turn_timer_ms: Some(Some(30_000)),
    };
    apply_ok(&mut r, fr(9, vec![DeltaOp::HostFacts(h2)]));
    assert_eq!(r.host_facts(), Some(&h2));
    // Neither op needs or creates a row.
    assert!(r.is_empty());
    // A snapshot on a replica that never saw them leaves them None.
    let mut fresh = rep();
    snap_ok(&mut fresh, p(0, 3, e0()));
    assert_eq!(fresh.pause(), None);
    assert_eq!(fresh.host_facts(), None);
}

// ---------------------------------------------------------------------------
// R7.
// ---------------------------------------------------------------------------

/// R-10, R-3: every snapshot check, its order and atomicity.
#[test]
fn r7_snapshot_checks_in_order() {
    let mut r = rep();
    apply_ok(&mut r, fr(1, vec![enter(1), enter(2)]));
    apply_ok(&mut r, fr(2, vec![enter(4), leave(4)]));
    // last 2, high-water 4, live {1, 2}.
    assert_eq!(r.next_event_seq(), 3);
    let cur = 2;

    // (a) Cursor, at the bound and one past it, both directions.
    refuse_snap(
        &mut r,
        p(1, 0, e0()),
        ReplicaError::SnapshotCursor { min: 2, got: 1 },
    );
    refuse_snap(
        &mut r,
        p(SEQ_LAST + 1, 0, e0()),
        ReplicaError::SnapshotCursor {
            min: 2,
            got: SEQ_LAST + 1,
        },
    );
    refuse_snap(
        &mut r,
        p(u64::MAX, 0, e0()),
        ReplicaError::SnapshotCursor {
            min: 2,
            got: u64::MAX,
        },
    );
    let c = control_snap(&r, p(cur, 0, body(rows(&[1, 2]), None, None)));
    assert_eq!(c.next_event_seq(), 3);
    let c = control_snap(&r, p(SEQ_LAST, 0, e0()));
    assert_eq!(c.next_event_seq(), u64::MAX);

    // (b) Row count.
    let many = |n: u64| rows(&(5..5 + n).collect::<Vec<_>>());
    assert_eq!(MAX_VISIBLE_ENTITIES, 1_024);
    refuse_snap(
        &mut r,
        p(cur, 0, body(many(1_025), None, None)),
        ReplicaError::TooManyEntities,
    );
    let c = control_snap(&r, p(cur, 0, body(many(1_024), None, None)));
    assert_eq!(c.len(), 1_024);
    assert_eq!(c.entity(id(1_028)), Some(&dflt(1_028)));
    assert_eq!(c.entity(id(1_029)), None);

    // (c) Order and reuse.
    refuse_snap(
        &mut r,
        p(cur, 0, body(rows(&[2, 1]), None, None)),
        ReplicaError::SnapshotOrder { entity: id(1) },
    );
    refuse_snap(
        &mut r,
        p(cur, 0, body(rows(&[1, 1]), None, None)),
        ReplicaError::SnapshotOrder { entity: id(1) },
    );
    refuse_snap(
        &mut r,
        p(cur, 0, body(rows(&[5, 4]), None, None)),
        ReplicaError::SnapshotOrder { entity: id(4) },
    );
    refuse_snap(
        &mut r,
        p(cur, 0, body(rows(&[4]), None, None)),
        ReplicaError::ReusedEntityId { entity: id(4) },
    );
    refuse_snap(
        &mut r,
        p(cur, 0, body(rows(&[1, 2, 4]), None, None)),
        ReplicaError::ReusedEntityId { entity: id(4) },
    );
    // Controls: live ids at or below the high-water are legal; a fresh id is.
    let c = control_snap(&r, p(cur, 0, body(rows(&[1, 2]), None, None)));
    assert_eq!(c.entities(), &[dflt(1), dflt(2)]);
    let c = control_snap(&r, p(cur, 0, body(rows(&[1, 5]), None, None)));
    assert_eq!(c.entities(), &[dflt(1), dflt(5)]);

    // (d) Order targets are body rows, not merely live rows.
    refuse_snap(
        &mut r,
        p(cur, 0, body(vec![rowf(1, fs_order(9)), row(2)], None, None)),
        unknown(9),
    );
    refuse_snap(
        &mut r,
        p(cur, 0, body(vec![rowf(1, fs_order(2))], None, None)),
        unknown(2),
    );
    let c = control_snap(
        &r,
        p(cur, 0, body(vec![rowf(1, fs_order(2)), row(2)], None, None)),
    );
    assert_eq!(c.entities(), &[ent(1, None, fs_order(2)), dflt(2)]);
    let c = control_snap(&r, p(cur, 0, body(vec![rowf(1, fs_order(1))], None, None)));
    assert_eq!(facts_of(&c, 1), fs_order(1));

    // (e) The active combatant is a body row.
    refuse_snap(
        &mut r,
        p(cur, 0, body(rows(&[1]), Some(turn(Some(2), 1)), None)),
        unknown(2),
    );
    let c = control_snap(
        &r,
        p(cur, 0, body(rows(&[1]), Some(turn(Some(1), 1)), None)),
    );
    assert_eq!(c.turn(), Some(&turn(Some(1), 1)));
    let c = control_snap(&r, p(cur, 0, body(rows(&[1]), Some(turn(None, 1)), None)));
    assert_eq!(c.turn(), Some(&turn(None, 1)));

    // (f) Timeline entries are body rows.
    refuse_snap(
        &mut r,
        p(
            cur,
            0,
            body(rows(&[1]), None, Some(vec![tl(0, 1), tl(0, 2)])),
        ),
        unknown(2),
    );
    let c = control_snap(
        &r,
        p(
            cur,
            0,
            body(rows(&[1, 2]), None, Some(vec![tl(0, 1), tl(0, 2)])),
        ),
    );
    assert_eq!(tl_vec(&c), Some(vec![tl(0, 1), tl(0, 2)]));

    // Precedence.
    refuse_snap(
        &mut r,
        p(1, 0, body(many(1_025), None, None)),
        ReplicaError::SnapshotCursor { min: 2, got: 1 },
    );
    let mut unordered = many(1_025);
    unordered.swap(0, 1);
    refuse_snap(
        &mut r,
        p(cur, 0, body(unordered, None, None)),
        ReplicaError::TooManyEntities,
    );
    refuse_snap(
        &mut r,
        p(cur, 0, body(vec![rowf(4, fs_order(9))], None, None)),
        ReplicaError::ReusedEntityId { entity: id(4) },
    );
    refuse_snap(
        &mut r,
        p(
            cur,
            0,
            body(vec![rowf(1, fs_order(9)), rowf(2, fs_order(8))], None, None),
        ),
        unknown(9),
    );
    refuse_snap(
        &mut r,
        p(
            cur,
            0,
            body(vec![rowf(1, fs_order(9))], Some(turn(Some(8), 1)), None),
        ),
        unknown(9),
    );
    refuse_snap(
        &mut r,
        p(
            cur,
            0,
            body(rows(&[1]), Some(turn(Some(8), 1)), Some(vec![tl(0, 7)])),
        ),
        unknown(8),
    );
    // The transfer id is not read.
    let mut other = p(cur, 0, body(rows(&[1, 2]), None, None));
    other.transfer = tid(999);
    let a = control_snap(&r, other);
    let b = control_snap(&r, p(cur, 0, body(rows(&[1, 2]), None, None)));
    assert_eq!(a, b);
    // On a fresh replica cursor 0 is the lowest legal one.
    let mut fresh = rep();
    snap_ok(&mut fresh, p(0, 5, body(rows(&[3]), None, None)));
    assert_eq!((fresh.next_event_seq(), fresh.server_tick()), (1, 5));
}

// ---------------------------------------------------------------------------
// R8.
// ---------------------------------------------------------------------------

/// R-11, R-12: a snapshot replaces every v3 field but host state.
#[test]
fn r8_snapshot_replaces_everything_but_host_state() {
    let mut r = rep();
    apply_ok(
        &mut r,
        fr(
            1,
            vec![
                enter(1),
                enter(2),
                enter(3),
                facts_op(1, fs_order(2)),
                DeltaOp::Encounter(ef_a()),
                timeline_op(Some(vec![tl(0, 1)])),
                turn_op(Some(3), 2),
                DeltaOp::Clock { paused: Some(None) },
            ],
        ),
    );
    let might = FactSet {
        stats: Some(vec![StatFact {
            name: "might".to_string(),
            value: StatValue::Int(3),
        }]),
        ..FactSet::default()
    };
    let only_order = FactSet {
        order: Some(None),
        ..FactSet::default()
    };
    let b = SnapshotBodyV3 {
        entities: vec![
            SnapshotEntityV3 {
                entity: id(1),
                health: Some(hf(2, 3, false)),
                facts: only_order.clone(),
            },
            row(3),
            rowf(5, might.clone()),
        ],
        turn: None,
        encounter: EncounterFacts::default(),
        timeline: Some(vec![tl(1, 5), tl(1, 1)]),
    };
    snap_ok(&mut r, p(5, 40, b));
    assert_eq!(
        r.entities(),
        &[
            ent(1, Some(hf(2, 3, false)), only_order),
            dflt(3),
            ent(5, None, might),
        ]
    );
    assert_eq!(r.len(), 3);
    assert_eq!(r.turn(), None);
    assert_eq!(*r.encounter(), EncounterFacts::default());
    assert_eq!(tl_vec(&r), Some(vec![tl(1, 5), tl(1, 1)]));
    assert_eq!(r.pause(), Some(PauseState::Paused { reason: None }));
    assert_eq!(r.next_event_seq(), 6);
    assert_eq!(r.server_tick(), 40);
    assert_eq!(r.entity(id(2)), None);
    assert_eq!(r.entity(id(4)), None);
    assert!(r.entity(id(5)).is_some());
    // High-water is 5: 2 and 4 are retired, 6 is fresh.
    refuse(
        &mut r,
        fr(6, vec![enter(2)]),
        ReplicaError::ReusedEntityId { entity: id(2) },
    );
    refuse(
        &mut r,
        fr(6, vec![enter(4)]),
        ReplicaError::ReusedEntityId { entity: id(4) },
    );
    apply_ok(&mut r, fr(6, vec![enter(6)]));
    assert_eq!(r.entity(id(6)), Some(&dflt(6)));
}

/// R-11: encounter and timeline come from the body, withheld included; the
/// high-water mark never goes down; `SEQ_LAST` closes the replica.
#[test]
fn r8_snapshot_replacement_details() {
    let mut r = rep();
    apply_ok(
        &mut r,
        fr(
            1,
            vec![
                enter(1),
                enter(7),
                DeltaOp::Encounter(ef_a()),
                timeline_op(Some(vec![tl(0, 1)])),
                turn_op(Some(1), 1),
            ],
        ),
    );
    // A body with a withheld timeline, a default encounter and no turn; its
    // last row (1) is below the high-water mark (7).
    snap_ok(&mut r, p(1, 3, body(rows(&[1]), None, None)));
    assert_eq!(r.timeline(), None);
    assert_eq!(*r.encounter(), EncounterFacts::default());
    assert_eq!(r.turn(), None);
    assert_eq!(r.entities(), &[dflt(1)]);
    // High-water stays 7: 7 is retired (no longer live), so is 3; 8 is fresh.
    refuse(
        &mut r,
        fr(2, vec![enter(7)]),
        ReplicaError::ReusedEntityId { entity: id(7) },
    );
    refuse(
        &mut r,
        fr(2, vec![enter(3)]),
        ReplicaError::ReusedEntityId { entity: id(3) },
    );
    let mut c = r.clone();
    apply_ok(&mut c, fr(2, vec![enter(8)]));
    // The body's encounter and an empty timeline are stored whole.
    let mut b = e0();
    b.encounter = ef_a();
    b.timeline = Some(vec![]);
    snap_ok(&mut r, p(1, 4, b));
    assert_eq!(*r.encounter(), ef_a());
    assert_eq!(r.timeline(), Some(&[][..]));
    // A cursor equal to the last applied sequence number is accepted, and
    // the next expected sequence number is cursor + 1.
    assert_eq!((r.next_event_seq(), r.server_tick()), (2, 4));
    assert!(r.is_empty());
    refuse(
        &mut r,
        fr(2, vec![enter(7)]),
        ReplicaError::ReusedEntityId { entity: id(7) },
    );
    // SEQ_LAST gives u64::MAX; nothing is accepted after it.
    snap_ok(&mut r, p(SEQ_LAST, 9, e0()));
    assert_eq!(r.next_event_seq(), u64::MAX);
    refuse(
        &mut r,
        frt(u64::MAX, 1, vec![]),
        ReplicaError::EventSeq {
            expected: u64::MAX,
            got: u64::MAX,
        },
    );
    // `last` is now SEQ_LAST: it is accepted again, one more is not.
    snap_ok(&mut r, p(SEQ_LAST, 10, e0()));
    assert_eq!(r.server_tick(), 10);
    refuse_snap(
        &mut r,
        p(SEQ_LAST + 1, 0, e0()),
        ReplicaError::SnapshotCursor {
            min: SEQ_LAST,
            got: SEQ_LAST + 1,
        },
    );
}

// ---------------------------------------------------------------------------
// R9.
// ---------------------------------------------------------------------------

/// R-3: a frame that fails on its last op leaves no trace of its first ones.
#[test]
fn r9_mid_frame_failure_changes_nothing() {
    let mut r = rep();
    apply_ok(&mut r, fr(1, vec![enter(1), enter(2)]));
    let good = vec![
        facts_op(1, fs_order(2)),
        DeltaOp::Encounter(ef_a()),
        timeline_op(Some(vec![tl(0, 1)])),
        DeltaOp::Clock { paused: Some(None) },
        DeltaOp::HostFacts(host_h()),
        enter(3),
        leave(1),
    ];
    let mut bad = good.clone();
    bad.push(facts_op(9, FactSet::default()));
    refuse(&mut r, fr(2, bad), unknown(9));
    assert_eq!(r.entities(), &[dflt(1), dflt(2)]);
    assert_eq!(*r.encounter(), EncounterFacts::default());
    assert_eq!(r.timeline(), None);
    assert_eq!(r.pause(), None);
    assert_eq!(r.host_facts(), None);
    assert_eq!((r.next_event_seq(), r.server_tick()), (2, 1));
    // The same frame without the last op shows every effect.
    apply_ok(&mut r, fr(2, good));
    assert_eq!(r.entities(), &[dflt(2), dflt(3)]);
    assert_eq!(*r.encounter(), ef_a());
    assert_eq!(r.timeline(), Some(&[tl(0, 1)][..]));
    assert_eq!(r.pause(), Some(PauseState::Paused { reason: None }));
    assert_eq!(r.host_facts(), Some(&host_h()));
    assert_eq!((r.next_event_seq(), r.server_tick()), (3, 2));
}

/// R-13: equal calls give equal replicas, and a refused call leaves a replica
/// equal to one that never saw it.
#[test]
fn r9_determinism_and_refusals_leave_no_trace() {
    let script = |with_refusals: bool| {
        let mut r = rep();
        apply_ok(&mut r, fr(1, vec![enter(1), enter(2), facts_op(1, fs_a())]));
        if with_refusals {
            assert_eq!(
                r.apply_delta(&fr(2, vec![enter(3), health_op(9, 1, 1, false)])),
                Err(unknown(9))
            );
            assert_eq!(
                r.apply_snapshot(&p(0, 3, e0())),
                Err(ReplicaError::SnapshotCursor { min: 1, got: 0 })
            );
        }
        apply_ok(&mut r, fr(2, vec![DeltaOp::Encounter(ef_a())]));
        snap_ok(&mut r, p(2, 8, body(rows(&[1, 2, 5]), None, None)));
        r
    };
    assert_eq!(script(false), script(false));
    assert_eq!(script(true), script(false));
}

// ---------------------------------------------------------------------------
// R10.
// ---------------------------------------------------------------------------

/// `d(1)`: an empty v3 delta frame with tick and sequence 1.
fn d1() -> Vec<u8> {
    codec_v3::encode_delta(&fr(1, vec![])).expect("d(1) encodes")
}

/// R-15, R-13: a pool feeds the replica; both agree on every v3 value.
#[test]
fn r10_pool_to_replica_end_to_end() {
    let bx = SnapshotBodyV3 {
        entities: vec![
            SnapshotEntityV3 {
                entity: id(1),
                health: Some(hf(10, 10, false)),
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
            row(2),
        ],
        turn: Some(turn(Some(2), 3)),
        encounter: EncounterFacts {
            mode: Some(EncounterMode::RealTime),
            pending_mode: Some(None),
            clock: Some(Some(ClockFact {
                round_ticks: 60,
                elapsed: 5,
            })),
        },
        timeline: Some(vec![tl(1, 2), tl(1, 1)]),
    };
    let frame3 = frt(
        3,
        51,
        vec![
            facts_op(1, FactSet::default()),
            DeltaOp::Clock {
                paused: Some(Some(PauseReason::Player)),
            },
        ],
    );
    let frame4 = frt(4, 52, vec![leave(2), timeline_op(Some(vec![tl(1, 1)]))]);

    let mut pool = ReassemblyPoolV3::new(TransferLimitsV3::v3()).expect("limits");
    let rx = pool
        .add_receiver(Z, GateMode::AwaitSnapshot)
        .expect("receiver");
    let now = 1_000;
    assert_eq!(pool.offer_delta(rx, &d1(), now), Ok(DeltaOutcome::Buffered));
    let bytes3 = codec_v3::encode_delta(&frame3).expect("encodes");
    assert_eq!(
        pool.offer_delta(rx, &bytes3, now),
        Ok(DeltaOutcome::Buffered)
    );
    let frames = t3::plan_transfer(Z, tid(1), 50, 2, 8, &bx).expect("plan");
    assert_eq!(frames.len(), 2);
    assert_eq!(
        pool.offer_transfer(rx, &frames[0], now),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        pool.offer_transfer(rx, &frames[1], now),
        Ok(TransferOutcome::Published {
            transfer: tid(1),
            released: 1
        })
    );
    let bytes4 = codec_v3::encode_delta(&frame4).expect("encodes");
    assert_eq!(
        pool.offer_delta(rx, &bytes4, now),
        Ok(DeltaOutcome::Released { count: 1 })
    );

    let expected_snapshot = PublicationV3 {
        transfer: tid(1),
        base_tick: 50,
        cursor: 2,
        body: bx.clone(),
    };
    let mut popped = Vec::new();
    while let Some(update) = pool.pop_update(rx) {
        popped.push(update);
    }
    assert_eq!(
        popped,
        vec![
            ReplicaUpdateV3::Snapshot(expected_snapshot.clone()),
            ReplicaUpdateV3::Delta(frame3.clone()),
            ReplicaUpdateV3::Delta(frame4.clone()),
        ]
    );
    assert_eq!(pool.pop_update(rx), None);

    let mut r = rep();
    for update in &popped {
        match update {
            ReplicaUpdateV3::Snapshot(publication) => {
                assert_eq!(r.apply_snapshot(publication), Ok(()));
                assert_eq!((r.next_event_seq(), r.server_tick()), (3, 50));
            }
            ReplicaUpdateV3::Delta(frame) => assert_eq!(r.apply_delta(frame), Ok(())),
        }
    }
    assert_eq!(
        r.entities(),
        &[ent(1, Some(hf(10, 10, false)), FactSet::default())]
    );
    assert_eq!(r.turn(), Some(&turn(Some(2), 3)));
    assert_eq!(*r.encounter(), bx.encounter);
    assert_eq!(r.encounter().pending_mode, Some(None));
    assert_eq!(tl_vec(&r), Some(vec![tl(1, 1)]));
    assert_eq!(
        r.pause(),
        Some(PauseState::Paused {
            reason: Some(PauseReason::Player)
        })
    );
    assert_eq!(r.host_facts(), None);
    assert_eq!((r.next_event_seq(), r.server_tick()), (5, 52));
    refuse(
        &mut r,
        fr(5, vec![enter(2)]),
        ReplicaError::ReusedEntityId { entity: id(2) },
    );

    // The same updates applied by hand give an equal replica (R-13).
    let mut by_hand = rep();
    snap_ok(&mut by_hand, expected_snapshot);
    apply_ok(&mut by_hand, frame3);
    apply_ok(&mut by_hand, frame4);
    assert_eq!(by_hand, r);
}

// ---------------------------------------------------------------------------
// Review survivors: per-row snapshot checks, enter order, Health isolation.
// ---------------------------------------------------------------------------

/// R-10(c): the checks run row by row, so a retired id in an early row is
/// reported as reuse even when a later row is out of order.
#[test]
fn r7_snapshot_order_and_reuse_are_checked_per_row() {
    let mut r = rep();
    // High-water 5, nothing live.
    apply_ok(&mut r, fr(1, vec![enter(5), leave(5)]));
    assert!(r.is_empty());
    // Row 2 is the first row: not above the (absent) previous row, and a
    // retired id. Reuse is reached first; row 1 is never examined.
    refuse_snap(
        &mut r,
        p(1, 0, body(rows(&[2, 1]), None, None)),
        ReplicaError::ReusedEntityId { entity: id(2) },
    );
    refuse_snap(
        &mut r,
        p(1, 0, body(rows(&[1, 2]), None, None)),
        ReplicaError::ReusedEntityId { entity: id(1) },
    );
    // Controls that differ in one input: fresh ids in order pass; fresh ids
    // out of order are an order fault at the second row.
    let c = control_snap(&r, p(1, 0, body(rows(&[6, 7]), None, None)));
    assert_eq!(c.entities(), &[dflt(6), dflt(7)]);
    refuse_snap(
        &mut r,
        p(1, 0, body(rows(&[7, 6]), None, None)),
        ReplicaError::SnapshotOrder { entity: id(6) },
    );
}

/// R-5: EntityEnter checks duplicate, then reuse, then the row bound.
#[test]
fn r2_enter_checks_reuse_before_the_bound() {
    let mut r = rep();
    for seq in 1..=8u64 {
        let first = (seq - 1) * 256 + 1;
        apply_ok(&mut r, fr(seq, (first..first + 256).map(enter).collect()));
    }
    assert_eq!(r.len(), 2_048);
    apply_ok(&mut r, fr(9, vec![leave(1), enter(2049)]));
    assert_eq!(r.len(), 2_048);
    // Full, and id 1 is retired: reuse wins over the bound.
    refuse(
        &mut r,
        fr(10, vec![enter(1)]),
        ReplicaError::ReusedEntityId { entity: id(1) },
    );
    // Duplicate wins over both.
    refuse(
        &mut r,
        fr(10, vec![enter(2)]),
        ReplicaError::DuplicateEntity { entity: id(2) },
    );
    // Control: a fresh id on the full replica meets the bound.
    refuse(
        &mut r,
        fr(10, vec![enter(2050)]),
        ReplicaError::TooManyEntities,
    );
}

/// R-5, R-6: Health replaces the health whole and never touches the facts.
#[test]
fn r3_health_leaves_facts_alone() {
    let mut r = rep();
    apply_ok(
        &mut r,
        fr(
            1,
            vec![
                enter(1),
                enter(2),
                health_op(1, 3, 10, false),
                facts_op(1, fs_a()),
            ],
        ),
    );
    apply_ok(&mut r, fr(2, vec![health_op(1, 7, 12, true)]));
    assert_eq!(facts_of(&r, 1), fs_a());
    assert_eq!(health_of(&r, 1), Some(Some(hf(7, 12, true))));
    assert_eq!(
        r.entity(id(1)),
        Some(&ent(1, Some(hf(7, 12, true)), fs_a()))
    );
    // Health on a row with default facts leaves them default, and a
    // later fact image leaves the health alone.
    apply_ok(&mut r, fr(3, vec![health_op(2, 1, 1, false)]));
    assert_eq!(facts_of(&r, 2), FactSet::default());
    apply_ok(&mut r, fr(4, vec![facts_op(2, fs_order(1))]));
    assert_eq!(health_of(&r, 2), Some(Some(hf(1, 1, false))));
}
