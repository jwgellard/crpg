//! Independent two-keys contract tests for T046 client replica.
//!
//! Derived from `tasks/T046.md` Exact contract only, through the public
//! `crpg-net` API. Each test names the contract Behaviour items it proves and
//! the R-case it implements. Expected values are hand-written literals or the
//! truth state of the test-local [`Script`] (own id counter rising from 1,
//! own visible set, turn and `event_seq`), never a second applier and never
//! production code. `InterestView` is not used. Every rejection compares the
//! replica with a pre-call `clone`, on a non-empty replica, not only on a
//! fresh one. Each case has its valid control.

use std::collections::{BTreeMap, BTreeSet};

use crpg_core::{DeterministicRng, Ulid};
use crpg_net::codec_transfer::{decode_body, encode_body};
use crpg_net::codec_v2::{decode_delta, encode_delta};
use crpg_net::protocol::{
    DeltaOp as OpV1, NetId, ReceiptStatus, SessionEpoch, LANE_COMBAT, MAX_DELTA_OPS,
    MAX_VISIBLE_ENTITIES, SEQ_LAST,
};
use crpg_net::protocol_transfer::{HealthFact, SnapshotBody, SnapshotEntity, TransferId, TurnFact};
use crpg_net::protocol_v2::{DeltaFrame as FrameV2, DeltaOp as OpV2};
use crpg_net::reassembly::Publication;
use crpg_net::replica::{ReplicaEntity, ReplicaError, ReplicaWorld, MAX_REPLICA_ENTITIES};

// ---------------------------------------------------------------------------
// Small fixtures.
// ---------------------------------------------------------------------------

fn epoch(byte: u8) -> SessionEpoch {
    SessionEpoch([byte; 16])
}

const EPOCH_BYTE: u8 = 0x2A;
const FOREIGN_BYTE: u8 = 0xF0;

fn ep_bytes() -> [u8; 16] {
    [EPOCH_BYTE; 16]
}

fn net(raw: u64) -> NetId {
    NetId::new(raw).expect("fixture id is nonzero")
}

fn tid(raw: u64) -> TransferId {
    TransferId::new(raw).expect("fixture transfer id is nonzero")
}

fn health(h: u32, m: u32, dead: bool) -> HealthFact {
    HealthFact {
        health: h,
        max_health: m,
        dead,
    }
}

fn turn(active: Option<u64>, round: u64) -> TurnFact {
    TurnFact {
        active: active.map(net),
        round,
    }
}

fn row(id: u64, h: Option<HealthFact>) -> ReplicaEntity {
    ReplicaEntity {
        net_id: net(id),
        health: h,
    }
}

fn frame(seq: u64, tick: u64, ops: Vec<OpV2>) -> FrameV2 {
    FrameV2 {
        lane: LANE_COMBAT,
        server_tick: tick,
        event_seq: seq,
        ops,
    }
}

fn enter(id: u64) -> OpV2 {
    OpV2::Legacy(OpV1::EntityEnter { entity: net(id) })
}

fn leave(id: u64) -> OpV2 {
    OpV2::Legacy(OpV1::EntityLeave { entity: net(id) })
}

fn despawn(id: u64) -> OpV2 {
    OpV2::Legacy(OpV1::Despawned { entity: net(id) })
}

fn spawned(id: u64) -> OpV2 {
    OpV2::Legacy(OpV1::Spawned { entity: net(id) })
}

fn died(id: u64) -> OpV2 {
    OpV2::Legacy(OpV1::Died { entity: net(id) })
}

fn health_op(id: u64, h: u32, m: u32, dead: bool) -> OpV2 {
    OpV2::Legacy(OpV1::Health {
        entity: net(id),
        health: h,
        max_health: m,
        dead,
    })
}

fn turn_op(active: Option<u64>, round: u64) -> OpV2 {
    OpV2::Legacy(OpV1::Turn {
        active: active.map(net),
        round,
    })
}

fn receipt_op(epoch_bytes: [u8; 16]) -> OpV2 {
    OpV2::Legacy(OpV1::Receipt {
        epoch: epoch_bytes,
        lane: LANE_COMBAT,
        seq: 1,
        processed_tick: 7,
        status: ReceiptStatus::Applied,
    })
}

fn action_op(actor: u64, target: u64) -> OpV2 {
    OpV2::ActionResolved {
        actor: net(actor),
        target: net(target),
        ability: Ulid::NIL,
        outcome: "success".to_owned(),
        damage: 0,
    }
}

fn turn_started(actor: u64, round: u64) -> OpV2 {
    OpV2::TurnStarted {
        actor: net(actor),
        round,
    }
}

fn ended() -> OpV2 {
    OpV2::EncounterEnded {
        encounter: Ulid::NIL,
        round: 0,
    }
}

fn body_of(rows: &[(u64, Option<HealthFact>)], t: Option<TurnFact>) -> SnapshotBody {
    SnapshotBody {
        entities: rows
            .iter()
            .map(|(id, h)| SnapshotEntity {
                entity: net(*id),
                health: h.clone(),
            })
            .collect(),
        turn: t,
    }
}

fn publication(cursor: u64, base_tick: u64, body: SnapshotBody) -> Publication {
    Publication {
        transfer: tid(1),
        base_tick,
        cursor,
        body,
    }
}

/// A non-empty replica holding enters 1 and 2 (seq 1..2 applied).
fn two_live() -> ReplicaWorld {
    let mut r = ReplicaWorld::new(epoch(EPOCH_BYTE));
    r.apply_delta(&frame(1, 10, vec![enter(1), enter(2)]))
        .expect("fixture enters apply");
    r
}

fn assert_clone_equal_after_err<T>(before: &T, after: &T)
where
    T: PartialEq + core::fmt::Debug,
{
    assert_eq!(before, after, "rejection leaves the replica unchanged");
}

// ---------------------------------------------------------------------------
// Test-local Script: server stand-in and truth oracle.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct Script {
    next_id: u64,
    live: BTreeMap<u64, Option<HealthFact>>,
    turn: Option<TurnFact>,
    event_seq: u64,
}

impl Script {
    fn new() -> Self {
        Self {
            next_id: 1,
            live: BTreeMap::new(),
            turn: None,
            event_seq: 1,
        }
    }

    fn mint(&mut self, n: usize) -> Vec<u64> {
        let mut out = Vec::new();
        for _ in 0..n {
            let id = self.next_id;
            self.next_id += 1;
            self.live.insert(id, None);
            out.push(id);
        }
        out
    }

    fn set_health(&mut self, id: u64, h: HealthFact) {
        *self.live.get_mut(&id).expect("script health on live") = Some(h);
    }

    fn set_turn(&mut self, t: Option<TurnFact>) {
        self.turn = t;
    }

    fn remove(&mut self, id: u64) {
        self.live.remove(&id).expect("script removes live");
    }

    fn live_ids(&self) -> Vec<u64> {
        self.live.keys().copied().collect()
    }

    fn entities(&self) -> Vec<ReplicaEntity> {
        self.live
            .iter()
            .map(|(id, h)| ReplicaEntity {
                net_id: net(*id),
                health: h.clone(),
            })
            .collect()
    }

    fn chunk(&mut self, ops: Vec<OpV2>, sizes: &[usize], tick_base: u64) -> Vec<FrameV2> {
        let mut frames = Vec::new();
        let mut at = 0;
        let mut which = 0;
        while at < ops.len() {
            let want = if sizes.is_empty() {
                ops.len() - at
            } else {
                sizes[which % sizes.len()]
            };
            which += 1;
            let take = want.max(1).min(ops.len() - at);
            let chunk = ops[at..at + take].to_vec();
            at += take;
            let seq = self.event_seq;
            self.event_seq += 1;
            frames.push(frame(seq, tick_base + seq, chunk));
        }
        if ops.is_empty() {
            let seq = self.event_seq;
            self.event_seq += 1;
            frames.push(frame(seq, tick_base + seq, vec![]));
        }
        frames
    }

    fn publication(&mut self, cursor: u64, base_tick: u64) -> Publication {
        // A snapshot body must carry an active inside the body (item
        // 11d): a stale active is rendered as no disclosed actor, and the
        // script truth follows so the oracle stays exact.
        let live_now: BTreeSet<u64> = self.live.keys().copied().collect();
        if let Some(t) = self.turn.clone() {
            if let Some(a) = t.active {
                if !live_now.contains(&a.get()) {
                    self.turn = Some(TurnFact {
                        active: None,
                        round: t.round,
                    });
                }
            }
        }
        let body = SnapshotBody {
            entities: self
                .live
                .iter()
                .map(|(id, h)| SnapshotEntity {
                    entity: net(*id),
                    health: h.clone(),
                })
                .collect(),
            turn: self.turn.clone(),
        };
        self.event_seq = cursor + 1;
        publication(cursor, base_tick, body)
    }

    fn last(&self) -> u64 {
        self.event_seq - 1
    }
}

fn assert_matches_script(r: &ReplicaWorld, s: &Script) {
    assert_eq!(r.entities(), s.entities().as_slice(), "rows match truth");
    assert_eq!(r.turn(), s.turn.as_ref(), "turn matches truth");
}

// ---------------------------------------------------------------------------
// R1 (items 1, 13). Construction.
// ---------------------------------------------------------------------------

#[test]
fn r1_construction_queries_and_first_frame() {
    let r = ReplicaWorld::new(epoch(EPOCH_BYTE));
    assert_eq!(r.epoch(), epoch(EPOCH_BYTE));
    assert!(r.entities().is_empty());
    assert_eq!(r.len(), 0);
    assert!(r.is_empty());
    assert_eq!(r.entity(net(1)), None);
    assert_eq!(r.turn(), None);
    assert_eq!(r.next_event_seq(), 1);
    assert_eq!(r.server_tick(), 0);
    // No second constructor: a fresh value equals a new one.
    assert_eq!(r, ReplicaWorld::new(epoch(EPOCH_BYTE)));
    assert_ne!(r, ReplicaWorld::new(epoch(FOREIGN_BYTE)));
    // Control: one frame gives len 1, next 2, tick 9.
    let mut r = r;
    r.apply_delta(&frame(1, 9, vec![enter(1)]))
        .expect("first frame applies");
    assert_eq!(r.entities(), &[row(1, None)]);
    assert_eq!(r.len(), 1);
    assert!(!r.is_empty());
    assert_eq!(r.entity(net(1)), Some(&row(1, None)));
    assert_eq!(r.next_event_seq(), 2);
    assert_eq!(r.server_tick(), 9);
}

// ---------------------------------------------------------------------------
// R2 (items 6, 9). Enter, health, leave, despawn.
// ---------------------------------------------------------------------------

#[test]
fn r2_enter_health_leave_despawn_run() {
    let mut r = ReplicaWorld::new(epoch(EPOCH_BYTE));
    r.apply_delta(&frame(1, 1, vec![enter(1), enter(2)]))
        .expect("enters apply");
    assert_eq!(r.entities(), &[row(1, None), row(2, None)]);
    r.apply_delta(&frame(2, 2, vec![health_op(1, 7, 10, false)]))
        .expect("health applies");
    assert_eq!(
        r.entities(),
        &[row(1, Some(health(7, 10, false))), row(2, None)]
    );
    r.apply_delta(&frame(3, 3, vec![leave(1)]))
        .expect("leave applies");
    assert_eq!(r.entities(), &[row(2, None)]);
    assert_eq!(r.entity(net(1)), None);
    r.apply_delta(&frame(4, 4, vec![despawn(2)]))
        .expect("despawn applies");
    assert!(r.entities().is_empty());
    assert!(r.is_empty());

    // Controls: the same run without one op leaves exactly that fact.
    let mut c = ReplicaWorld::new(epoch(EPOCH_BYTE));
    c.apply_delta(&frame(1, 1, vec![enter(1), enter(2)]))
        .expect("control enters");
    c.apply_delta(&frame(2, 2, vec![]))
        .expect("control skips health");
    assert_eq!(c.entities(), &[row(1, None), row(2, None)]);
    let mut c = ReplicaWorld::new(epoch(EPOCH_BYTE));
    c.apply_delta(&frame(1, 1, vec![enter(1), enter(2)]))
        .expect("control enters");
    c.apply_delta(&frame(2, 2, vec![health_op(1, 7, 10, false)]))
        .expect("control health");
    c.apply_delta(&frame(3, 3, vec![]))
        .expect("control skips leave");
    assert_eq!(
        c.entities(),
        &[row(1, Some(health(7, 10, false))), row(2, None)]
    );
    let mut c = ReplicaWorld::new(epoch(EPOCH_BYTE));
    for (seq, ops) in [
        (1, vec![enter(1), enter(2)]),
        (2, vec![health_op(1, 7, 10, false)]),
        (3, vec![leave(1)]),
    ] {
        c.apply_delta(&frame(seq, seq, ops))
            .expect("control prefix");
    }
    c.apply_delta(&frame(4, 4, vec![]))
        .expect("control skips despawn");
    assert_eq!(c.entities(), &[row(2, None)]);
}

// ---------------------------------------------------------------------------
// R3 (item 7, N4). Unknown is not zero.
// ---------------------------------------------------------------------------

#[test]
fn r3_unknown_health_is_none_not_zero() {
    let mut r = ReplicaWorld::new(epoch(EPOCH_BYTE));
    r.apply_delta(&frame(1, 1, vec![enter(1), enter(2)]))
        .expect("enters apply");
    assert_eq!(r.entity(net(1)), Some(&row(1, None)));
    r.apply_delta(&frame(2, 2, vec![health_op(1, 0, 10, false)]))
        .expect("zero health applies");
    assert_eq!(r.entity(net(1)), Some(&row(1, Some(health(0, 10, false)))));
    assert_eq!(r.entity(net(99)), None, "never-entered id is None");
    // Control: the zero row and an untouched row compare unequal.
    assert_ne!(r.entity(net(1)), r.entity(net(2)));
    assert_eq!(r.entity(net(2)), Some(&row(2, None)));
}

// ---------------------------------------------------------------------------
// R4 (item 7). Death.
// ---------------------------------------------------------------------------

#[test]
fn r4_death_keeps_row_and_died_changes_nothing() {
    let mut r = ReplicaWorld::new(epoch(EPOCH_BYTE));
    r.apply_delta(&frame(1, 1, vec![enter(1), enter(2)]))
        .expect("enters apply");
    r.apply_delta(&frame(2, 2, vec![health_op(1, 4, 10, false)]))
        .expect("alive health applies");
    // Died leaves health unchanged, for a None row and an alive Some row.
    r.apply_delta(&frame(3, 3, vec![died(1), died(2)]))
        .expect("died notices apply");
    assert_eq!(r.entity(net(1)), Some(&row(1, Some(health(4, 10, false)))));
    assert_eq!(r.entity(net(2)), Some(&row(2, None)));
    assert_eq!(r.len(), 2, "dead rows stay");
    // Control: a Health with dead true is what sets it.
    r.apply_delta(&frame(4, 4, vec![health_op(1, 4, 10, true)]))
        .expect("dead health applies");
    assert_eq!(r.entity(net(1)), Some(&row(1, Some(health(4, 10, true)))));
    assert_eq!(r.len(), 2, "dead flag never removes the row");
}

// ---------------------------------------------------------------------------
// R5 (item 6). Notices, events, receipts are inert.
// ---------------------------------------------------------------------------

#[test]
fn r5_notices_events_receipts_inert_and_faults() {
    let mut r = ReplicaWorld::new(epoch(EPOCH_BYTE));
    r.apply_delta(&frame(1, 1, vec![enter(1), enter(2)]))
        .expect("enters apply");
    r.apply_delta(&frame(2, 2, vec![health_op(1, 5, 10, false)]))
        .expect("health applies");
    let before_rows = r.entities().to_vec();
    let before_turn = r.turn().cloned();
    r.apply_delta(&frame(
        3,
        33,
        vec![
            spawned(1),
            died(2),
            action_op(1, 2),
            turn_started(1, 4),
            ended(),
            receipt_op(ep_bytes()),
        ],
    ))
    .expect("inert ops apply");
    assert_eq!(r.entities(), before_rows.as_slice());
    assert_eq!(r.turn(), before_turn.as_ref());
    assert_eq!(r.next_event_seq(), 4);
    assert_eq!(r.server_tick(), 33);

    // Controls on a non-empty replica: each id-bearing op with a non-live
    // id gives UnknownEntity with that id (item 6 checks notices too).
    let before = r.clone();
    assert_eq!(
        r.apply_delta(&frame(4, 4, vec![spawned(77)])),
        Err(ReplicaError::UnknownEntity { entity: net(77) }),
        "Spawned over unknown id"
    );
    assert_clone_equal_after_err(&before, &r);
    assert_eq!(
        r.apply_delta(&frame(4, 4, vec![died(77)])),
        Err(ReplicaError::UnknownEntity { entity: net(77) }),
        "Died over unknown id"
    );
    assert_clone_equal_after_err(&before, &r);
    assert_eq!(
        r.apply_delta(&frame(4, 4, vec![action_op(77, 1)])),
        Err(ReplicaError::UnknownEntity { entity: net(77) }),
        "actor unknown first"
    );
    assert_clone_equal_after_err(&before, &r);
    assert_eq!(
        r.apply_delta(&frame(4, 4, vec![action_op(1, 78)])),
        Err(ReplicaError::UnknownEntity { entity: net(78) }),
        "live actor then unknown target"
    );
    assert_clone_equal_after_err(&before, &r);
    assert_eq!(
        r.apply_delta(&frame(4, 4, vec![turn_started(79, 0)])),
        Err(ReplicaError::UnknownEntity { entity: net(79) }),
    );
    assert_clone_equal_after_err(&before, &r);
    assert_eq!(
        r.apply_delta(&frame(4, 4, vec![receipt_op([FOREIGN_BYTE; 16])])),
        Err(ReplicaError::WrongEpoch),
    );
    assert_clone_equal_after_err(&before, &r);
    // Control: the right frame still applies afterwards.
    r.apply_delta(&frame(4, 4, vec![receipt_op(ep_bytes())]))
        .expect("own-epoch receipt applies");
    assert_eq!(r.next_event_seq(), 5);
}

// ---------------------------------------------------------------------------
// R6 (items 6, 8, N6, N7). Turn.
// ---------------------------------------------------------------------------

#[test]
fn r6_turn_stored_verbatim_and_kept() {
    let mut r = ReplicaWorld::new(epoch(EPOCH_BYTE));
    r.apply_delta(&frame(1, 1, vec![enter(1), enter(2)]))
        .expect("enters apply");
    // Live active stored verbatim.
    r.apply_delta(&frame(2, 2, vec![turn_op(Some(1), 3)]))
        .expect("turn applies");
    assert_eq!(r.turn(), Some(&turn(Some(1), 3)));
    // Active None stored as-is.
    r.apply_delta(&frame(3, 3, vec![turn_op(None, 4)]))
        .expect("empty active applies");
    assert_eq!(r.turn(), Some(&turn(None, 4)));
    // Active entered earlier in the same frame is legal.
    r.apply_delta(&frame(4, 4, vec![enter(3), turn_op(Some(3), 5)]))
        .expect("enter-then-turn applies");
    assert_eq!(r.turn(), Some(&turn(Some(3), 5)));
    // Active entered later in the same frame is UnknownEntity.
    let before = r.clone();
    assert_eq!(
        r.apply_delta(&frame(5, 5, vec![turn_op(Some(4), 6), enter(4)])),
        Err(ReplicaError::UnknownEntity { entity: net(4) }),
    );
    assert_clone_equal_after_err(&before, &r);
    // Removed actor in Turn is UnknownEntity.
    assert_eq!(
        r.apply_delta(&frame(5, 5, vec![turn_op(Some(9), 6)])),
        Err(ReplicaError::UnknownEntity { entity: net(9) }),
    );
    assert_clone_equal_after_err(&before, &r);
    // After the actor leaves, and after EncounterEnded, turn is unchanged.
    r.apply_delta(&frame(5, 5, vec![leave(3)]))
        .expect("actor leaves");
    assert_eq!(r.turn(), Some(&turn(Some(3), 5)), "turn keeps stale active");
    r.apply_delta(&frame(6, 6, vec![ended()]))
        .expect("end applies");
    assert_eq!(r.turn(), Some(&turn(Some(3), 5)), "end never clears turn");
    // Control: a new Turn replaces it.
    r.apply_delta(&frame(7, 7, vec![turn_op(Some(1), 8)]))
        .expect("replacement applies");
    assert_eq!(r.turn(), Some(&turn(Some(1), 8)));
}

// ---------------------------------------------------------------------------
// R7 (items 2c, 4, 5). Sequence.
// ---------------------------------------------------------------------------

#[test]
fn r7_sequence_law_gaps_rewinds_and_ticks() {
    // Frames 1, 2 (empty) and 3 (notices only) are accepted.
    let mut r = ReplicaWorld::new(epoch(EPOCH_BYTE));
    r.apply_delta(&frame(1, 10, vec![enter(1)]))
        .expect("frame 1 applies");
    r.apply_delta(&frame(2, 11, vec![])).expect("empty applies");
    assert_eq!(r.next_event_seq(), 3);
    r.apply_delta(&frame(3, 12, vec![spawned(1), died(1)]))
        .expect("notices-only applies");
    assert_eq!(r.next_event_seq(), 4);
    assert_eq!(r.server_tick(), 12);

    // Each fault below reports the exact EventSeq and changes nothing,
    // on a non-empty replica.
    let faulty: Vec<(FrameV2, u64, u64)> = vec![
        (frame(2, 20, vec![]), 1, 2), // placeholder, replaced below
    ];
    let _ = faulty;
    // First frame at 2 on a fresh replica.
    let mut f = ReplicaWorld::new(epoch(EPOCH_BYTE));
    f.apply_delta(&frame(1, 9, vec![enter(5)]))
        .expect("non-empty setup");
    let _before_f = f.clone();
    assert_eq!(
        ReplicaWorld::new(epoch(EPOCH_BYTE)).apply_delta(&frame(2, 1, vec![])),
        Err(ReplicaError::EventSeq {
            expected: 1,
            got: 2
        }),
    );
    // Gap 1 then 3.
    let mut g = ReplicaWorld::new(epoch(EPOCH_BYTE));
    g.apply_delta(&frame(1, 1, vec![])).expect("frame 1");
    let before = g.clone();
    assert_eq!(
        g.apply_delta(&frame(3, 1, vec![])),
        Err(ReplicaError::EventSeq {
            expected: 2,
            got: 3
        }),
    );
    assert_clone_equal_after_err(&before, &g);
    // Duplicate.
    assert_eq!(
        g.apply_delta(&frame(1, 1, vec![])),
        Err(ReplicaError::EventSeq {
            expected: 2,
            got: 1
        }),
    );
    assert_clone_equal_after_err(&before, &g);
    // Rewind after 1, 2.
    g.apply_delta(&frame(2, 1, vec![])).expect("frame 2");
    let before = g.clone();
    assert_eq!(
        g.apply_delta(&frame(1, 1, vec![])),
        Err(ReplicaError::EventSeq {
            expected: 3,
            got: 1
        }),
    );
    assert_clone_equal_after_err(&before, &g);
    // Restart at 1 after 3.
    g.apply_delta(&frame(3, 1, vec![])).expect("frame 3");
    let before = g.clone();
    assert_eq!(
        g.apply_delta(&frame(1, 1, vec![])),
        Err(ReplicaError::EventSeq {
            expected: 4,
            got: 1
        }),
    );
    assert_clone_equal_after_err(&before, &g);
    // Zero and above SEQ_LAST.
    assert_eq!(
        ReplicaWorld::new(epoch(EPOCH_BYTE)).apply_delta(&frame(0, 1, vec![])),
        Err(ReplicaError::EventSeq {
            expected: 1,
            got: 0
        }),
    );
    let mut h = two_live();
    let before = h.clone();
    assert_eq!(
        h.apply_delta(&frame(u64::MAX, 1, vec![])),
        Err(ReplicaError::EventSeq {
            expected: 2,
            got: u64::MAX
        }),
    );
    assert_clone_equal_after_err(&before, &h);
    assert_eq!(
        ReplicaWorld::new(epoch(EPOCH_BYTE)).apply_delta(&frame(u64::MAX, 1, vec![])),
        Err(ReplicaError::EventSeq {
            expected: 1,
            got: u64::MAX
        }),
    );
    // Non-empty clone check for the fresh-replica faults above.
    let before = f.clone();
    assert_eq!(
        f.apply_delta(&frame(0, 1, vec![])),
        Err(ReplicaError::EventSeq {
            expected: 2,
            got: 0
        }),
    );
    assert_clone_equal_after_err(&before, &f);
    // After a publication at SEQ_LAST, frame u64::MAX gives {MAX, MAX}.
    let mut p = two_live();
    let body = SnapshotBody {
        entities: vec![SnapshotEntity {
            entity: net(1),
            health: None,
        }],
        turn: None,
    };
    p.apply_snapshot(&publication(SEQ_LAST, 77, body))
        .expect("publication at SEQ_LAST applies");
    assert_eq!(p.next_event_seq(), u64::MAX);
    let before = p.clone();
    assert_eq!(
        p.apply_delta(&frame(u64::MAX, 1, vec![])),
        Err(ReplicaError::EventSeq {
            expected: u64::MAX,
            got: u64::MAX
        }),
    );
    assert_clone_equal_after_err(&before, &p);
    // A backwards tick is accepted.
    let mut t = ReplicaWorld::new(epoch(EPOCH_BYTE));
    t.apply_delta(&frame(1, 100, vec![])).expect("tick 100");
    t.apply_delta(&frame(2, 50, vec![]))
        .expect("backwards tick");
    assert_eq!(t.server_tick(), 50);
    // Controls: after each fault the right frame applies, and a new
    // replica accepts the restarted stream from 1.
    g.apply_delta(&frame(4, 1, vec![]))
        .expect("right frame after faults");
    assert_eq!(g.next_event_seq(), 5);
    let mut fresh = ReplicaWorld::new(epoch(EPOCH_BYTE));
    fresh
        .apply_delta(&frame(1, 1, vec![enter(1)]))
        .expect("restarted stream from 1");
    assert_eq!(fresh.len(), 1);
}

// ---------------------------------------------------------------------------
// R8 (items 2a, 2b). Shape.
// ---------------------------------------------------------------------------

#[test]
fn r8_lane_and_op_count_shape() {
    assert_eq!(MAX_DELTA_OPS, 256);
    let mut r = two_live();
    let before = r.clone();
    assert_eq!(
        r.apply_delta(&FrameV2 {
            lane: 1,
            server_tick: 1,
            event_seq: 3,
            ops: vec![],
        }),
        Err(ReplicaError::WrongLane { got: 1 }),
    );
    assert_clone_equal_after_err(&before, &r);
    let many: Vec<OpV2> = (0..257).map(|_| died(1)).collect();
    assert_eq!(
        r.apply_delta(&frame(3, 1, many)),
        Err(ReplicaError::TooManyOps { got: 257 }),
    );
    assert_clone_equal_after_err(&before, &r);
    // Control: a 256-op lane-0 frame applies.
    let mut c = ReplicaWorld::new(epoch(EPOCH_BYTE));
    let enters: Vec<OpV2> = (1..=256).map(enter).collect();
    c.apply_delta(&frame(1, 5, enters)).expect("256 ops apply");
    assert_eq!(c.len(), 256);
    assert_eq!(c.next_event_seq(), 2);
}

// ---------------------------------------------------------------------------
// R9 (item 6, D6, N3). Duplicate and reuse.
// ---------------------------------------------------------------------------

#[test]
fn r9_duplicate_and_reuse() {
    let mut r = ReplicaWorld::new(epoch(EPOCH_BYTE));
    r.apply_delta(&frame(1, 1, vec![enter(1), enter(2), enter(3), enter(5)]))
        .expect("enters apply");
    // Entering a live id is DuplicateEntity.
    let before = r.clone();
    assert_eq!(
        r.apply_delta(&frame(2, 2, vec![enter(2)])),
        Err(ReplicaError::DuplicateEntity { entity: net(2) }),
    );
    assert_clone_equal_after_err(&before, &r);
    // Leaving 2 then entering 2 is ReusedEntityId.
    r.apply_delta(&frame(2, 2, vec![leave(2)]))
        .expect("leave 2 applies");
    let before = r.clone();
    assert_eq!(
        r.apply_delta(&frame(3, 3, vec![enter(2)])),
        Err(ReplicaError::ReusedEntityId { entity: net(2) }),
    );
    assert_clone_equal_after_err(&before, &r);
    // Never-seen 4 below high-water 5 is ReusedEntityId.
    assert_eq!(
        r.apply_delta(&frame(3, 3, vec![enter(4)])),
        Err(ReplicaError::ReusedEntityId { entity: net(4) }),
    );
    assert_clone_equal_after_err(&before, &r);
    // Same-frame leave-then-reenter is reuse too.
    assert_eq!(
        r.apply_delta(&frame(3, 3, vec![leave(1), enter(1)])),
        Err(ReplicaError::ReusedEntityId { entity: net(1) }),
    );
    assert_clone_equal_after_err(&before, &r);
    // Controls: entering 6 applies; a new replica accepts entering 2.
    r.apply_delta(&frame(3, 3, vec![enter(6)]))
        .expect("enter above high-water applies");
    assert!(r.entity(net(6)).is_some());
    let mut fresh = ReplicaWorld::new(epoch(EPOCH_BYTE));
    fresh
        .apply_delta(&frame(1, 1, vec![enter(2)]))
        .expect("fresh replica accepts 2");
}

// ---------------------------------------------------------------------------
// R10 (item 10, D8). Bounds.
// ---------------------------------------------------------------------------

#[test]
fn r10_mid_page_bound_and_swap() {
    assert_eq!(MAX_VISIBLE_ENTITIES, 1024);
    assert_eq!(MAX_REPLICA_ENTITIES, 2048);
    // 2,048 enters across 8 frames apply; the 2,049th is refused.
    let mut r = ReplicaWorld::new(epoch(EPOCH_BYTE));
    for f in 0..8 {
        let ops: Vec<OpV2> = (1..=256).map(|i| enter(f * 256 + i)).collect();
        r.apply_delta(&frame(f + 1, f + 1, ops))
            .expect("fill frame applies");
    }
    assert_eq!(r.len(), 2048);
    let before = r.clone();
    assert_eq!(
        r.apply_delta(&frame(9, 9, vec![enter(2049)])),
        Err(ReplicaError::TooManyEntities),
    );
    assert_clone_equal_after_err(&before, &r);
    // Control: at 2,048, a leave then an enter in one frame applies.
    r.apply_delta(&frame(9, 9, vec![leave(1), enter(2049)]))
        .expect("leave-then-enter at cap applies");
    assert_eq!(r.len(), 2048);
    assert!(r.entity(net(2049)).is_some());

    // Swap page over 1,024 live rows: 1,024 enters then 1,024 leaves.
    let mut s = ReplicaWorld::new(epoch(EPOCH_BYTE));
    for f in 0..4 {
        let ops: Vec<OpV2> = (1..=256).map(|i| enter(f * 256 + i)).collect();
        s.apply_delta(&frame(f + 1, f + 1, ops)).expect("base fill");
    }
    assert_eq!(s.len(), 1024);
    for f in 0..4 {
        let ops: Vec<OpV2> = (1..=256).map(|i| enter(1024 + f * 256 + i)).collect();
        s.apply_delta(&frame(5 + f, 5 + f, ops))
            .expect("swap enter");
    }
    assert_eq!(s.len(), 2048, "mid-page peak after frame 4 of swap");
    for f in 0..4 {
        let ops: Vec<OpV2> = (1..=256).map(|i| leave(f * 256 + i)).collect();
        s.apply_delta(&frame(9 + f, 9 + f, ops))
            .expect("swap leave");
    }
    assert_eq!(s.len(), 1024);
    assert!(s.entity(net(1)).is_none());
    assert!(s.entity(net(1025)).is_some());
    assert!(s.entity(net(2048)).is_some());
}

// ---------------------------------------------------------------------------
// R11 (item 12, N5). Replacement.
// ---------------------------------------------------------------------------

#[test]
fn r11_snapshot_replacement_and_empty_body() {
    let mut r = ReplicaWorld::new(epoch(EPOCH_BYTE));
    r.apply_delta(&frame(
        1,
        10,
        vec![enter(1), enter(2), enter(3), health_op(1, 9, 9, false)],
    ))
    .expect("base applies");
    r.apply_delta(&frame(2, 11, vec![turn_op(Some(3), 7)]))
        .expect("turn applies");
    let last = 2;
    // Body {1, 3, 4} with health only on 4 and turn None.
    let body = body_of(
        &[(1, None), (3, None), (4, Some(health(2, 8, false)))],
        None,
    );
    r.apply_snapshot(&publication(last, 55, body))
        .expect("snapshot applies");
    assert_eq!(
        r.entities(),
        &[
            row(1, None),
            row(3, None),
            row(4, Some(health(2, 8, false)))
        ],
        "row 1 loses old health; row 2 gone"
    );
    assert_eq!(r.turn(), None);
    assert_eq!(r.server_tick(), 55);
    assert_eq!(r.next_event_seq(), last + 1);
    // A later op on removed 2 is UnknownEntity.
    let before = r.clone();
    assert_eq!(
        r.apply_delta(&frame(last + 1, 56, vec![health_op(2, 1, 1, false)])),
        Err(ReplicaError::UnknownEntity { entity: net(2) }),
    );
    assert_clone_equal_after_err(&before, &r);
    // Controls: next delta at cursor + 1 applies; 2 is reuse; 5 is fresh.
    r.apply_delta(&frame(last + 1, 56, vec![health_op(4, 3, 8, false)]))
        .expect("delta after snapshot applies");
    assert_eq!(r.entity(net(4)), Some(&row(4, Some(health(3, 8, false)))));
    assert_eq!(
        r.apply_delta(&frame(last + 2, 57, vec![enter(2)])),
        Err(ReplicaError::ReusedEntityId { entity: net(2) }),
    );
    r.apply_delta(&frame(last + 2, 57, vec![enter(5)]))
        .expect("enter above high-water applies");

    // Empty body on the same shape empties rows, keeps high-water.
    let mut e = ReplicaWorld::new(epoch(EPOCH_BYTE));
    e.apply_delta(&frame(1, 1, vec![enter(1), enter(2), enter(3)]))
        .expect("base applies");
    e.apply_delta(&frame(2, 2, vec![turn_op(Some(3), 7)]))
        .expect("turn applies");
    e.apply_snapshot(&publication(2, 60, body_of(&[], None)))
        .expect("empty body applies");
    assert_eq!(e.len(), 0);
    assert!(e.is_empty());
    assert_eq!(e.turn(), None);
    assert_eq!(e.next_event_seq(), 3);
    let before = e.clone();
    assert_eq!(
        e.apply_delta(&frame(3, 61, vec![enter(3)])),
        Err(ReplicaError::ReusedEntityId { entity: net(3) }),
        "high-water kept through empty body"
    );
    assert_clone_equal_after_err(&before, &e);
    e.apply_delta(&frame(3, 61, vec![enter(5)]))
        .expect("enter above high-water applies");
}

// ---------------------------------------------------------------------------
// R12 (items 11a, 12). Cursor.
// ---------------------------------------------------------------------------

#[test]
fn r12_snapshot_cursor_advances_and_skips() {
    // Cursor 0 on a new replica gives 1.
    let mut r = ReplicaWorld::new(epoch(EPOCH_BYTE));
    r.apply_snapshot(&publication(0, 10, body_of(&[], None)))
        .expect("cursor 0 on new replica");
    assert_eq!(r.next_event_seq(), 1);

    // After frames 1..3: cursor 3 gives 4, cursor 7 gives 8 (skip).
    let mut r = ReplicaWorld::new(epoch(EPOCH_BYTE));
    for seq in 1..=3 {
        r.apply_delta(&frame(seq, seq, vec![]))
            .expect("empty frame");
    }
    r.apply_snapshot(&publication(3, 40, body_of(&[], None)))
        .expect("cursor 3 applies");
    assert_eq!(r.next_event_seq(), 4);
    r.apply_snapshot(&publication(7, 41, body_of(&[], None)))
        .expect("cursor 7 skips");
    assert_eq!(r.next_event_seq(), 8);

    // Cursor 2 is SnapshotCursor{3, 2} on a 1..3 replica.
    let mut r = ReplicaWorld::new(epoch(EPOCH_BYTE));
    for seq in 1..=3 {
        r.apply_delta(&frame(seq, seq, vec![]))
            .expect("empty frame");
    }
    let before = r.clone();
    assert_eq!(
        r.apply_snapshot(&publication(2, 40, body_of(&[], None))),
        Err(ReplicaError::SnapshotCursor { min: 3, got: 2 }),
    );
    assert_clone_equal_after_err(&before, &r);
    assert_eq!(
        r.apply_snapshot(&publication(SEQ_LAST + 1, 40, body_of(&[], None))),
        Err(ReplicaError::SnapshotCursor {
            min: 3,
            got: SEQ_LAST + 1
        }),
    );
    assert_clone_equal_after_err(&before, &r);
    // Control: frame cursor + 1 applies, cursor + 2 and cursor are EventSeq.
    let mut r = ReplicaWorld::new(epoch(EPOCH_BYTE));
    for seq in 1..=3 {
        r.apply_delta(&frame(seq, seq, vec![]))
            .expect("empty frame");
    }
    r.apply_snapshot(&publication(7, 41, body_of(&[], None)))
        .expect("cursor 7 applies");
    r.apply_delta(&frame(8, 42, vec![]))
        .expect("cursor + 1 applies");
    assert_eq!(
        r.apply_delta(&frame(10, 43, vec![])),
        Err(ReplicaError::EventSeq {
            expected: 9,
            got: 10
        }),
    );
    assert_eq!(
        r.apply_delta(&frame(8, 43, vec![])),
        Err(ReplicaError::EventSeq {
            expected: 9,
            got: 8
        }),
    );
}

// ---------------------------------------------------------------------------
// R13 (item 11). Snapshot faults.
// ---------------------------------------------------------------------------

#[test]
fn r13_snapshot_faults_and_order() {
    let mut r = two_live();
    r.apply_delta(&frame(2, 2, vec![leave(2)]))
        .expect("retire 2 for reuse setup");
    // High-water is 2 here; extend it to 5 with never-seen 4 skipped.
    r.apply_delta(&frame(3, 3, vec![enter(3), enter(5)]))
        .expect("extend high-water to 5");
    let last = 3;
    // 1,025 rows.
    let big: Vec<(u64, Option<HealthFact>)> =
        (10_000..10_000 + 1025).map(|id| (id, None)).collect();
    let before = r.clone();
    assert_eq!(
        r.apply_snapshot(&publication(last, 1, body_of(&big, None))),
        Err(ReplicaError::TooManyEntities),
    );
    assert_clone_equal_after_err(&before, &r);
    // Order: fresh ids above high-water, so only the order rule fires.
    for rows in [
        vec![(6002, None), (6001, None)],
        vec![(6001, None), (6001, None)],
    ] {
        assert_eq!(
            r.apply_snapshot(&publication(last, 1, body_of(&rows, None))),
            Err(ReplicaError::SnapshotOrder { entity: net(6001) }),
        );
        assert_clone_equal_after_err(&before, &r);
    }
    // Order beats reuse (item 11c lists order first): row 2 below the
    // previous row 3 is SnapshotOrder even though 2 is also retired.
    assert_eq!(
        r.apply_snapshot(&publication(
            last,
            1,
            body_of(&[(3, None), (2, None)], None)
        )),
        Err(ReplicaError::SnapshotOrder { entity: net(2) }),
    );
    assert_clone_equal_after_err(&before, &r);
    // Retired id and never-seen id below high-water.
    assert_eq!(
        r.apply_snapshot(&publication(last, 1, body_of(&[(2, None)], None))),
        Err(ReplicaError::ReusedEntityId { entity: net(2) }),
    );
    assert_clone_equal_after_err(&before, &r);
    assert_eq!(
        r.apply_snapshot(&publication(last, 1, body_of(&[(4, None)], None))),
        Err(ReplicaError::ReusedEntityId { entity: net(4) }),
    );
    assert_clone_equal_after_err(&before, &r);
    // Active outside the body.
    assert_eq!(
        r.apply_snapshot(&publication(
            last,
            1,
            body_of(&[(1, None)], Some(turn(Some(99), 1)))
        )),
        Err(ReplicaError::UnknownEntity { entity: net(99) }),
    );
    assert_clone_equal_after_err(&before, &r);
    // Combined faults report in item 11 order: cursor first, then count,
    // then order, then reuse, then turn.
    assert_eq!(
        r.apply_snapshot(&publication(last - 1, 1, body_of(&big, None))),
        Err(ReplicaError::SnapshotCursor {
            min: last,
            got: last - 1
        }),
    );
    assert_clone_equal_after_err(&before, &r);
    let big_bad: Vec<(u64, Option<HealthFact>)> =
        (10_000..10_000 + 1025).rev().map(|id| (id, None)).collect();
    assert_eq!(
        r.apply_snapshot(&publication(last, 1, body_of(&big_bad, None))),
        Err(ReplicaError::TooManyEntities),
    );
    assert_clone_equal_after_err(&before, &r);
    assert_eq!(
        r.apply_snapshot(&publication(
            last,
            1,
            body_of(&[(3, None), (2, None)], Some(turn(Some(99), 1)))
        )),
        Err(ReplicaError::SnapshotOrder { entity: net(2) }),
    );
    assert_clone_equal_after_err(&before, &r);
    assert_eq!(
        r.apply_snapshot(&publication(
            last,
            1,
            body_of(&[(2, None)], Some(turn(Some(99), 1)))
        )),
        Err(ReplicaError::ReusedEntityId { entity: net(2) }),
    );
    assert_clone_equal_after_err(&before, &r);
    // Controls: 1,024 ascending rows apply; ids above high-water apply;
    // an active inside the body applies.
    let ok: Vec<(u64, Option<HealthFact>)> = (10_000..10_000 + 1024).map(|id| (id, None)).collect();
    r.apply_snapshot(&publication(last, 9, body_of(&ok, None)))
        .expect("1,024 rows apply");
    assert_eq!(r.len(), 1024);
    let mut s = two_live();
    s.apply_snapshot(&publication(
        2,
        9,
        body_of(&[(6, None), (7, None)], Some(turn(Some(7), 2))),
    ))
    .expect("fresh ids and inside active apply");
    assert_eq!(s.turn(), Some(&turn(Some(7), 2)));
}

// ---------------------------------------------------------------------------
// R14 (item 3). Atomicity.
// ---------------------------------------------------------------------------

#[test]
fn r14_atomicity_all_variants_and_multi_op_frame() {
    let mut r = two_live();
    r.apply_delta(&frame(2, 30, vec![enter(3), health_op(1, 1, 5, false)]))
        .expect("non-empty setup");
    // All ten variants on this non-empty replica (next is 3, last is 2).
    let big: Vec<(u64, Option<HealthFact>)> =
        (50_000..50_000 + 1025).map(|id| (id, None)).collect();
    let many: Vec<OpV2> = (0..257).map(|_| died(1)).collect();
    let bad_lane = FrameV2 {
        lane: 1,
        server_tick: 1,
        event_seq: 3,
        ops: vec![],
    };
    let delta_cases: Vec<FrameV2> = vec![
        bad_lane,
        frame(3, 1, many),
        frame(99, 1, vec![]),
        frame(3, 1, vec![health_op(77, 1, 1, false)]),
        frame(3, 1, vec![enter(1)]),
        frame(3, 1, vec![enter(2)]),
        frame(3, 1, vec![health_op(1, 1, 1, false), leave(1), enter(1)]),
        frame(3, 1, vec![receipt_op([FOREIGN_BYTE; 16])]),
    ];
    for f in &delta_cases {
        let before = r.clone();
        assert!(r.apply_delta(f).is_err(), "delta fault applies nothing");
        assert_clone_equal_after_err(&before, &r);
    }
    // The 9th delta case above covers reuse-via-leave; TooManyEntities
    // needs a full replica, checked on its own clone below.
    let snapshot_cases: Vec<(Publication, ReplicaError)> = vec![
        (
            publication(1, 1, body_of(&big, None)),
            ReplicaError::TooManyEntities,
        ),
        (
            publication(4, 1, body_of(&[(3, None), (2, None)], None)),
            ReplicaError::SnapshotOrder { entity: net(2) },
        ),
        (
            publication(1, 1, body_of(&[], None)),
            ReplicaError::SnapshotCursor { min: 2, got: 1 },
        ),
    ];
    for (p, _want) in &snapshot_cases {
        let before = r.clone();
        assert!(
            r.apply_snapshot(p).is_err(),
            "snapshot fault applies nothing"
        );
        assert_clone_equal_after_err(&before, &r);
    }
    // One frame [Enter 9, Leave 1, Health 2, Turn{9}, Enter 9] fails on
    // op 5 and changes nothing.
    let mut s = ReplicaWorld::new(epoch(EPOCH_BYTE));
    s.apply_delta(&frame(1, 1, vec![enter(1), enter(2)]))
        .expect("base");
    let before = s.clone();
    assert_eq!(
        s.apply_delta(&frame(
            2,
            2,
            vec![
                enter(9),
                leave(1),
                health_op(2, 8, 8, false),
                turn_op(Some(9), 1),
                enter(9),
            ]
        )),
        Err(ReplicaError::DuplicateEntity { entity: net(9) }),
    );
    assert_clone_equal_after_err(&before, &s);
    // Control: the same frame without op 5 applies with all four effects.
    s.apply_delta(&frame(
        2,
        2,
        vec![
            enter(9),
            leave(1),
            health_op(2, 8, 8, false),
            turn_op(Some(9), 1),
        ],
    ))
    .expect("prefix applies");
    assert_eq!(s.entity(net(1)), None);
    assert_eq!(s.entity(net(9)), Some(&row(9, None)));
    assert_eq!(s.entity(net(2)), Some(&row(2, Some(health(8, 8, false)))));
    assert_eq!(s.turn(), Some(&turn(Some(9), 1)));
}

// ---------------------------------------------------------------------------
// R15 (item 4, N8). Split pages.
// ---------------------------------------------------------------------------

#[test]
fn r15_split_pages_converge() {
    let mut script = Script::new();
    let minted = script.mint(3);
    assert_eq!(minted, vec![1, 2, 3]);
    let h1 = health(6, 10, false);
    let h2 = health(3, 9, false);
    script.set_health(1, h1.clone());
    script.set_health(2, h2.clone());
    script.set_turn(Some(turn(Some(2), 5)));
    let page: Vec<OpV2> = vec![
        enter(1),
        enter(2),
        enter(3),
        spawned(3),
        died(1),
        health_op(1, 6, 10, false),
        health_op(2, 3, 9, false),
        turn_op(Some(2), 5),
    ];
    // Three applications: one frame, one-op frames, and uneven chunks.
    let mut a = ReplicaWorld::new(epoch(EPOCH_BYTE));
    a.apply_delta(&frame(1, 11, page.clone()))
        .expect("unsplit applies");
    let mut b = ReplicaWorld::new(epoch(EPOCH_BYTE));
    for (i, op) in page.clone().into_iter().enumerate() {
        b.apply_delta(&frame(i as u64 + 1, 11, vec![op]))
            .expect("one-op frame applies");
    }
    // After the enter frames of the split run, rows are live unknown.
    let mut probe = ReplicaWorld::new(epoch(EPOCH_BYTE));
    for (i, op) in [enter(1), enter(2), enter(3)].into_iter().enumerate() {
        probe
            .apply_delta(&frame(i as u64 + 1, 11, vec![op]))
            .expect("enter frame applies");
    }
    assert_eq!(
        probe.entities(),
        &[row(1, None), row(2, None), row(3, None)]
    );
    let mut c = ReplicaWorld::new(epoch(EPOCH_BYTE));
    let mut at = 0;
    for (k, size) in [2usize, 3, 1, 2].iter().enumerate() {
        let seq = k as u64 + 1;
        let take = (*size).min(page.len() - at);
        c.apply_delta(&frame(seq, 11, page[at..at + take].to_vec()))
            .expect("chunk applies");
        at += take;
    }
    assert_eq!(a.entities(), b.entities());
    assert_eq!(a.turn(), b.turn());
    assert_eq!(a.entities(), c.entities());
    assert_eq!(a.turn(), c.turn());
    assert_eq!(a.entities(), script.entities().as_slice());
    assert_eq!(a.turn(), script.turn.as_ref());
    // Control: the unsplit page is the reference above (a).
    assert_matches_script(&a, &script);
}

// ---------------------------------------------------------------------------
// R16 (item 14). Scripted convergence.
// ---------------------------------------------------------------------------

#[test]
fn r16_scripted_convergence_two_chunkings() {
    let mut rng = DeterministicRng::from_seed(46);
    let stream = rng.stream("replica");
    let mut script = Script::new();
    let mut a = ReplicaWorld::new(epoch(EPOCH_BYTE));
    let mut b = ReplicaWorld::new(epoch(EPOCH_BYTE));
    // Seed a small base so later steps can name live ids.
    for id in script.mint(4) {
        let _ = id;
    }
    let base_ops: Vec<OpV2> = vec![enter(1), enter(2), enter(3), enter(4)];
    let base_frames = script.chunk(base_ops, &[4], 1_000);
    assert_eq!(base_frames.len(), 1);
    for f in &base_frames {
        a.apply_delta(f).expect("base applies to A");
        b.apply_delta(f).expect("base applies to B");
    }
    assert_matches_script(&a, &script);

    let seen_live: BTreeSet<u64> = script.live_ids().into_iter().collect();
    let _ = seen_live;
    for step in 0..2_000u32 {
        let pick = stream.next_u32() % 10;
        let live: Vec<u64> = script.live_ids();
        let mut ops: Vec<OpV2> = Vec::new();
        match pick {
            0 | 1 => {
                // Enters, a few at a time.
                let n = (stream.next_u32() % 4 + 1) as usize;
                for id in script.mint(n) {
                    ops.push(enter(id));
                }
            }
            2 => {
                // Healths on live ids, both value directions.
                for _ in 0..(stream.next_u32() % 3 + 1) {
                    if live.is_empty() {
                        break;
                    }
                    let id = live[(stream.next_u32() as usize) % live.len()];
                    let h = health(
                        stream.next_u32() % 12,
                        10,
                        stream.next_u32().is_multiple_of(5),
                    );
                    script.set_health(id, h.clone());
                    ops.push(health_op(id, h.health, h.max_health, h.dead));
                }
            }
            3 => {
                // Leaves or despawns of live ids.
                let n = (stream.next_u32() % 3 + 1) as usize;
                for _ in 0..n {
                    let cur: Vec<u64> = script.live_ids();
                    if cur.is_empty() {
                        break;
                    }
                    let id = cur[(stream.next_u32() as usize) % cur.len()];
                    let is_despawn = stream.next_u32().is_multiple_of(2);
                    script.remove(id);
                    ops.push(if is_despawn { despawn(id) } else { leave(id) });
                }
            }
            4 => {
                // Notices and events over live ids.
                if !live.is_empty() {
                    let id = live[(stream.next_u32() as usize) % live.len()];
                    match stream.next_u32() % 5 {
                        0 => ops.push(spawned(id)),
                        1 => ops.push(died(id)),
                        2 => {
                            let other = live[(stream.next_u32() as usize) % live.len()];
                            ops.push(action_op(id, other));
                        }
                        3 => ops.push(turn_started(id, (stream.next_u32() % 20) as u64)),
                        _ => ops.push(ended()),
                    }
                }
                ops.push(receipt_op(ep_bytes()));
            }
            5 => {
                // Turn changes: live active both ways, None, kept stale.
                match stream.next_u32() % 3 {
                    0 if !live.is_empty() => {
                        let id = live[(stream.next_u32() as usize) % live.len()];
                        let t = turn(Some(id), (stream.next_u32() % 30) as u64);
                        script.set_turn(Some(t.clone()));
                        ops.push(turn_op(Some(id), t.round));
                    }
                    1 => {
                        let t = turn(None, (stream.next_u32() % 30) as u64);
                        script.set_turn(Some(t.clone()));
                        ops.push(turn_op(None, t.round));
                    }
                    _ => {
                        ops.push(ended());
                    }
                }
            }
            6 => {
                // Swap: enter as many as currently live, then remove old.
                let cur: Vec<u64> = script.live_ids();
                if !cur.is_empty() && cur.len() + (script.next_id as usize) < 100_000 {
                    let n = cur.len().min(24);
                    let mut fresh = Vec::new();
                    for id in script.mint(n) {
                        ops.push(enter(id));
                        fresh.push(id);
                    }
                    let _ = fresh;
                    for id in cur.into_iter().take(n) {
                        script.remove(id);
                        ops.push(leave(id));
                    }
                }
            }
            7 => {
                // Publication, sometimes skipping frames ahead.
                let skip = (stream.next_u32() % 4) as u64;
                let cursor = script.last() + skip;
                if cursor <= SEQ_LAST {
                    let base = 2_000 + step as u64;
                    let pub_now = script.publication(cursor, base);
                    a.apply_snapshot(&pub_now).expect("snapshot applies");
                    b.apply_snapshot(&pub_now).expect("snapshot applies");
                    assert_matches_script(&a, &script);
                    assert_matches_script(&b, &script);
                    assert_eq!(a, b);
                    continue;
                }
            }
            _ => {
                // Mixed page in T027b order: enters, events, healths, turn.
                for id in script.mint((stream.next_u32() % 3) as usize) {
                    ops.push(enter(id));
                }
                let cur: Vec<u64> = script.live_ids();
                if !cur.is_empty() {
                    let id = cur[(stream.next_u32() as usize) % cur.len()];
                    ops.push(died(id));
                    let h = health(stream.next_u32() % 12, 12, false);
                    script.set_health(id, h.clone());
                    ops.push(health_op(id, h.health, h.max_health, h.dead));
                    if stream.next_u32().is_multiple_of(2) {
                        let t = turn(Some(id), step as u64);
                        script.set_turn(Some(t.clone()));
                        ops.push(turn_op(Some(id), t.round));
                    }
                }
            }
        }
        if ops.is_empty() {
            ops.push(receipt_op(ep_bytes()));
        }
        // Cap guard: keep pages small so the bound is never the story
        // here (R10 proves the bound itself).
        ops.truncate(MAX_DELTA_OPS);
        // Two chunkings with equal frame counts but different split
        // points, so both waterlines agree: A [1, n-1], B [n-1, 1].
        let n = ops.len();
        let (sizes_a, sizes_b): (Vec<usize>, Vec<usize>) = if n < 2 {
            (vec![1], vec![1])
        } else {
            (vec![1, n - 1], vec![n - 1, 1])
        };
        let fa = script.chunk(ops.clone(), &sizes_a, 5_000);
        let mut fb = Vec::new();
        {
            let mut at = 0;
            for (i, want) in sizes_b.iter().enumerate() {
                let take = (*want).min(n - at);
                if take == 0 {
                    continue;
                }
                fb.push(frame(
                    fa[i].event_seq,
                    fa[i].server_tick,
                    ops[at..at + take].to_vec(),
                ));
                at += take;
            }
            assert_eq!(at, n);
            assert_eq!(fb.len(), fa.len());
        }
        for f in &fa {
            a.apply_delta(f).expect("page applies to A");
        }
        // B applies the same page content chunked differently; its
        // expected next seq is the same waterline.
        for f in &fb {
            b.apply_delta(f).expect("page applies to B");
        }
        // Advance B over any remainder the two-split skipped: both
        // chunkings cover the same ops, so both waterlines agree.
        assert_eq!(a.next_event_seq(), b.next_event_seq());
        assert_matches_script(&a, &script);
        assert_matches_script(&b, &script);
        assert_eq!(a, b);
    }
    // Control: identical inputs are equal under PartialEq, and one
    // changed health value makes them unequal.
    assert_eq!(a, b);
    let c = a.clone();
    assert_eq!(a, c);
    if let Some(first) = c.entities().first().cloned() {
        let flipped = ReplicaEntity {
            net_id: first.net_id,
            health: Some(health(999, 999, true)),
        };
        let mut rows = c.entities().to_vec();
        rows[0] = flipped;
        let other = ReplicaWorld::new(epoch(EPOCH_BYTE));
        let _ = other;
        assert_ne!(rows.as_slice(), a.entities());
    }
}

// ---------------------------------------------------------------------------
// R17 (corrupt and drop).
// ---------------------------------------------------------------------------

#[test]
fn r17_drops_swaps_and_reorders_fail_loudly() {
    // Build a three-page run through the Script truth.
    let mut script = Script::new();
    let mut pages: Vec<Vec<OpV2>> = Vec::new();
    for id in script.mint(3) {
        let _ = id;
    }
    pages.push(vec![enter(1), enter(2), enter(3)]);
    script.set_health(1, health(4, 10, false));
    pages.push(vec![health_op(1, 4, 10, false), turn_op(Some(2), 1)]);
    pages.push(vec![action_op(1, 2), leave(3)]);
    script.remove(3);
    script.set_turn(Some(turn(Some(2), 1)));
    let mut frames: Vec<FrameV2> = Vec::new();
    let mut seq = 1;
    for (i, ops) in pages.into_iter().enumerate() {
        frames.push(frame(seq, 100 + i as u64, ops));
        seq += 1;
    }
    script.event_seq = seq;
    // Control: untouched pages converge.
    let mut good = ReplicaWorld::new(epoch(EPOCH_BYTE));
    for f in &frames {
        good.apply_delta(f).expect("control page applies");
    }
    assert_matches_script(&good, &script);

    // Dropping a frame gives EventSeq on the next apply.
    let mut r = ReplicaWorld::new(epoch(EPOCH_BYTE));
    r.apply_delta(&frames[0]).expect("first applies");
    let before = r.clone();
    assert_eq!(
        r.apply_delta(&frames[2]),
        Err(ReplicaError::EventSeq {
            expected: 2,
            got: 3
        }),
    );
    assert_clone_equal_after_err(&before, &r);
    // Swapping two frames gives EventSeq.
    let mut r = ReplicaWorld::new(epoch(EPOCH_BYTE));
    r.apply_delta(&frames[0]).expect("first applies");
    assert_eq!(
        r.apply_delta(&frames[2]),
        Err(ReplicaError::EventSeq {
            expected: 2,
            got: 3
        }),
    );
    // Moving a Health before its enter gives UnknownEntity.
    let mut r = ReplicaWorld::new(epoch(EPOCH_BYTE));
    let before = r.clone();
    assert_eq!(
        r.apply_delta(&frame(1, 1, vec![health_op(9, 1, 5, false), enter(9)])),
        Err(ReplicaError::UnknownEntity { entity: net(9) }),
    );
    assert_clone_equal_after_err(&before, &r);
    // Duplicating a removal gives UnknownEntity.
    let mut r = ReplicaWorld::new(epoch(EPOCH_BYTE));
    r.apply_delta(&frame(1, 1, vec![enter(1)])).expect("enter");
    r.apply_delta(&frame(2, 2, vec![leave(1)])).expect("leave");
    let before = r.clone();
    assert_eq!(
        r.apply_delta(&frame(3, 3, vec![leave(1)])),
        Err(ReplicaError::UnknownEntity { entity: net(1) }),
    );
    assert_clone_equal_after_err(&before, &r);
    // Altering one health value is accepted but fails the truth
    // comparison, proving the oracle is not vacuous.
    let mut r = ReplicaWorld::new(epoch(EPOCH_BYTE));
    for f in &frames {
        r.apply_delta(f).expect("replay applies");
    }
    let mut tampered = frames[1].clone();
    tampered.ops[0] = health_op(1, 5, 10, false);
    let mut t = ReplicaWorld::new(epoch(EPOCH_BYTE));
    t.apply_delta(&frames[0]).expect("first applies");
    t.apply_delta(&tampered)
        .expect("altered health still applies");
    t.apply_delta(&frames[2]).expect("third applies");
    assert_ne!(t.entities(), script.entities().as_slice());
}

// ---------------------------------------------------------------------------
// R18 (wire impact).
// ---------------------------------------------------------------------------

#[test]
fn r18_codec_round_trips_match_struct_paths() {
    // A literal page through encode/decode then apply equals struct path.
    let ops: Vec<OpV2> = vec![
        enter(1),
        enter(2),
        health_op(1, 6, 10, false),
        turn_op(Some(2), 3),
        action_op(1, 2),
    ];
    let literal = frame(1, 44, ops);
    let bytes = encode_delta(&literal).expect("literal encodes");
    let decoded = decode_delta(&bytes).expect("literal decodes");
    assert_eq!(decoded, literal);
    let mut via_struct = ReplicaWorld::new(epoch(EPOCH_BYTE));
    via_struct
        .apply_delta(&literal)
        .expect("struct path applies");
    let mut via_wire = ReplicaWorld::new(epoch(EPOCH_BYTE));
    via_wire.apply_delta(&decoded).expect("wire path applies");
    assert_eq!(via_struct, via_wire);
    assert_eq!(
        via_wire.entities(),
        &[row(1, Some(health(6, 10, false))), row(2, None)]
    );
    // A body through encode_body/decode_body then Publication equals the
    // struct path.
    let body = body_of(
        &[(1, Some(health(6, 10, false))), (2, None)],
        Some(turn(Some(2), 3)),
    );
    let body_bytes = encode_body(&body).expect("body encodes");
    let decoded_body = decode_body(&body_bytes).expect("body decodes");
    assert_eq!(decoded_body, body);
    let mut a = two_live();
    let mut b = two_live();
    a.apply_snapshot(&publication(2, 90, body.clone()))
        .expect("struct snapshot applies");
    b.apply_snapshot(&publication(2, 90, decoded_body))
        .expect("wire snapshot applies");
    assert_eq!(a, b);
    // A frame with event_seq u64::MAX round-trips the codec and is
    // refused here.
    let maxed = frame(u64::MAX, 1, vec![]);
    let max_bytes = encode_delta(&maxed).expect("max seq encodes");
    assert_eq!(decode_delta(&max_bytes).expect("max seq decodes"), maxed);
    let mut r = ReplicaWorld::new(epoch(EPOCH_BYTE));
    let before = r.clone();
    assert_eq!(
        r.apply_delta(&maxed),
        Err(ReplicaError::EventSeq {
            expected: 1,
            got: u64::MAX
        }),
    );
    assert_clone_equal_after_err(&before, &r);
}

// ---------------------------------------------------------------------------
// R19 (item 15). Errors.
// ---------------------------------------------------------------------------

#[test]
fn r19_error_display_and_traits() {
    fn show(e: ReplicaError) -> String {
        e.to_string()
    }
    assert_eq!(
        show(ReplicaError::WrongLane { got: 1 }),
        "WrongLane at replica: got 1"
    );
    assert_eq!(
        show(ReplicaError::TooManyOps { got: 257 }),
        "TooManyOps at replica: got 257"
    );
    assert_eq!(
        show(ReplicaError::EventSeq {
            expected: 4,
            got: 7
        }),
        "EventSeq at replica: expected 4, got 7"
    );
    assert_eq!(
        show(ReplicaError::UnknownEntity { entity: net(9) }),
        "UnknownEntity at replica: 9"
    );
    assert_eq!(
        show(ReplicaError::DuplicateEntity { entity: net(3) }),
        "DuplicateEntity at replica: 3"
    );
    assert_eq!(
        show(ReplicaError::ReusedEntityId { entity: net(2) }),
        "ReusedEntityId at replica: 2"
    );
    assert_eq!(
        show(ReplicaError::TooManyEntities),
        "TooManyEntities at replica"
    );
    assert_eq!(show(ReplicaError::WrongEpoch), "WrongEpoch at replica");
    assert_eq!(
        show(ReplicaError::SnapshotOrder { entity: net(5) }),
        "SnapshotOrder at replica: 5"
    );
    assert_eq!(
        show(ReplicaError::SnapshotCursor { min: 3, got: 9 }),
        "SnapshotCursor at replica: min 3, got 9"
    );
    fn assert_copy<T: Copy>(v: T) -> T {
        v
    }
    let err = ReplicaError::WrongEpoch;
    let _ = assert_copy(err);
    fn assert_error<T: std::error::Error>(_: &T) {}
    assert_error(&err);
    let boxed: &dyn std::error::Error = &err;
    assert!(boxed.source().is_none());
}
