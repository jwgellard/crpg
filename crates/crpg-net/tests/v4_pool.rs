//! T026p4 implementer suite: the protocol v4 receiver gate (`pool_v4`),
//! derived from `tasks/T026p.md` Behaviour V-1..V-5 and the tests V1..V5,
//! through the public API only.
//!
//! `ReassemblyPoolV4` is `ReassemblyPoolV3` transliterated onto v4 deltas, so
//! the v3 pool is the oracle: the `Mirror` harness drives one script on a v3
//! and a v4 pool of the same limits (same transfer bytes; each delta's v3
//! bytes with byte 0 changed from 3 to 4) and compares every call's result,
//! every receiver's whole status, both pool totals, and every popped update
//! (a v4 `Delta` against the v3 frame with its ops wrapped in `V3`). It also
//! checks that every refusal that is not `Failed(_)` leaves both pools equal
//! to their pre-call clones when the call's clock does not advance the
//! pool's. The step lists of V1 and V5 are transcribed from `tasks/T140r.md`
//! (P8, then P2, P3, P4, P5, P9, P10, P11), not from `tests/v3_pool.rs`; the
//! D5 body builder is copied, never imported.

use std::sync::OnceLock;

use crpg_core::Ulid;
use crpg_net::codec::CodecError;
use crpg_net::codec_v3::encode_delta as encode_delta_v3;
use crpg_net::codec_v4::encode_delta as encode_delta_v4;
use crpg_net::pool_v3::{ReassemblyPoolV3, ReceiverIdV3, ReplicaUpdateV3};
use crpg_net::pool_v4::{
    DeltaOutcome, FailReason, GateMode, InFlightStatus, LimitsError, PublicationV3, PublishedBase,
    ReassemblyPoolV4, Rebound, ReceiverIdV4, ReceiverStatusV3, ReplicaUpdateV4, ResyncReason,
    TransferError, TransferLimitsV3, TransferOutcome,
};
use crpg_net::protocol::SEQ_LAST;
use crpg_net::protocol_transfer::{
    BeginTransfer, CancelReason, TransferChunk, TransferDown, TransferId,
};
use crpg_net::protocol_v3::{
    ClockFact, DeltaFrame as DeltaFrameV3, DeltaOp as DeltaOpV3, EffectFact, EncounterFacts,
    EncounterMode, FactSet, NetId, OrderFact, PoolFact, StatFact, StatValue, TimelineEntry,
};
use crpg_net::protocol_v4::{DeltaFrame, DeltaOp, Point, PositionFact};
use crpg_net::transfer_v3::{
    encode_down, plan_transfer, HealthFact, SnapshotBodyV3, SnapshotEntityV3, TurnFact,
};

// ---------------------------------------------------------------------------
// Fixtures.
// ---------------------------------------------------------------------------

const Z16: [u8; 16] = [0; 16];
const CHUNK: usize = 32_768;

fn tid(n: u64) -> TransferId {
    TransferId::new(n).expect("fixture transfer id is nonzero")
}

fn id(n: u64) -> NetId {
    NetId::new(n).expect("fixture net id is nonzero")
}

fn u(n: u128) -> Ulid {
    Ulid::from_u128(n)
}

fn p(x: i32, y: i32, z: i32) -> Point {
    Point::new([x, y, z]).expect("fixture point is in bounds")
}

/// `A` and `B` of the contract.
fn pa() -> Point {
    p(0, 0, 0)
}

fn pb() -> Point {
    p(65_536, 0, -131_072)
}

/// `L3`.
fn l3() -> TransferLimitsV3 {
    TransferLimitsV3::v3()
}

/// The contract's `d(n)` in its v3 form: `03 00 nn nn 00`, `n <= 127`.
fn d(n: u64) -> Vec<u8> {
    assert!(n <= 127);
    vec![0x03, 0x00, n as u8, n as u8, 0x00]
}

/// The decoded v3 form of `d(n)`.
fn dframe(n: u64) -> DeltaFrameV3 {
    DeltaFrameV3 {
        lane: 0,
        server_tick: n,
        event_seq: n,
        ops: vec![],
    }
}

/// The contract's `E0`: the empty v3 body, 6 bytes.
fn e0() -> SnapshotBodyV3 {
    SnapshotBodyV3 {
        entities: vec![],
        turn: None,
        encounter: EncounterFacts::default(),
        timeline: None,
    }
}

/// The contract's `plan(t, base, cursor, segs, body)`.
fn plan(t: u64, base: u64, cursor: u64, segs: u8, body: &SnapshotBodyV3) -> Vec<Vec<u8>> {
    plan_transfer(Z16, tid(t), base, cursor, segs, body).expect("fixture transfer plans")
}

fn begin_frame(begin: BeginTransfer) -> Vec<u8> {
    encode_down(&TransferDown::Begin(begin)).expect("fixture Begin encodes")
}

/// The contract's `G(t)` with a chosen cursor (`G(t)` is cursor 0).
fn g_at(t: u64, cursor: u64) -> Vec<u8> {
    begin_frame(BeginTransfer {
        epoch: Z16,
        transfer: tid(t),
        base_tick: 0,
        cursor,
        total_bytes: 4_194_304,
        entity_count: 0,
        segments_per_chunk: 8,
        chunk_count: 128,
    })
}

fn g(t: u64) -> Vec<u8> {
    g_at(t, 0)
}

/// The contract's `g(t, i)`: a Chunk of 32,768 bytes of `i`.
fn g_chunk(t: u64, i: u8) -> Vec<u8> {
    encode_down(&TransferDown::Chunk(TransferChunk {
        epoch: Z16,
        transfer: tid(t),
        index: i,
        bytes: vec![i; CHUNK],
    }))
    .expect("fixture chunk encodes")
}

/// The contract's `B(t, total)`.
fn b_frame(t: u64, total: u32) -> Vec<u8> {
    begin_frame(BeginTransfer {
        epoch: Z16,
        transfer: tid(t),
        base_tick: 0,
        cursor: 0,
        total_bytes: total,
        entity_count: 0,
        segments_per_chunk: 8,
        chunk_count: (total as usize).div_ceil(CHUNK) as u8,
    })
}

fn cancel_for(epoch: [u8; 16], t: u64) -> Vec<u8> {
    encode_down(&TransferDown::Cancel {
        epoch,
        transfer: tid(t),
        reason: CancelReason::Superseded,
    })
    .expect("fixture Cancel encodes")
}

/// P-3's literal for a fresh receiver.
fn fresh(mode: GateMode) -> ReceiverStatusV3 {
    ReceiverStatusV3 {
        epoch: Z16,
        mode,
        in_flight: None,
        retired_through: 0,
        published: None,
        next_event_seq: 1,
        held_frames: 0,
        held_bytes: 0,
        queued_updates: 0,
        resync: None,
    }
}

fn conflict() -> TransferError {
    TransferError::Failed(FailReason::Conflict)
}

fn unsupported() -> TransferError {
    TransferError::Codec(CodecError::UnsupportedVersion)
}

/// A v3 delta's bytes as the v4 wire has them: byte 0 changed from 3 to 4.
fn to_v4(v3_bytes: &[u8]) -> Vec<u8> {
    assert_eq!(v3_bytes[0], 0x03, "not a v3 delta");
    let mut out = v3_bytes.to_vec();
    out[0] = 0x04;
    out
}

/// A popped v3 update as the v4 pool must have released it: the same
/// snapshot, or the same frame with every op wrapped in `V3`.
fn wrap(update: &ReplicaUpdateV3) -> ReplicaUpdateV4 {
    match update {
        ReplicaUpdateV3::Snapshot(publication) => ReplicaUpdateV4::Snapshot(publication.clone()),
        ReplicaUpdateV3::Delta(frame) => ReplicaUpdateV4::Delta(DeltaFrame {
            lane: frame.lane,
            server_tick: frame.server_tick,
            event_seq: frame.event_seq,
            ops: frame.ops.iter().cloned().map(DeltaOp::V3).collect(),
        }),
    }
}

/// D5's largest body (2,847,788 bytes), copied from the contract's text.
fn largest_body() -> SnapshotBodyV3 {
    let base = 1u64 << 63;
    let entities = (0..1024u64)
        .map(|i| SnapshotEntityV3 {
            entity: id(base + i),
            health: Some(HealthFact {
                health: u32::MAX,
                max_health: u32::MAX,
                dead: true,
            }),
            facts: FactSet {
                pools: Some(
                    (1..=16)
                        .map(|n| PoolFact {
                            pool: u(n),
                            current: u32::MAX,
                            max: u32::MAX,
                        })
                        .collect(),
                ),
                effects: Some(
                    (1..=16)
                        .map(|n| EffectFact {
                            effect: u(n),
                            expires_round: u64::MAX,
                        })
                        .collect(),
                ),
                order: Some(Some(OrderFact {
                    ability: u(1),
                    target: id(base),
                    repeat: true,
                })),
                stats: Some(
                    (0..32)
                        .map(|n| StatFact {
                            name: format!("{n:032}"),
                            value: StatValue::Int(i32::MIN),
                        })
                        .collect(),
                ),
                tags: Some((0..8).map(|n| format!("{n:032}")).collect()),
            },
        })
        .collect();
    SnapshotBodyV3 {
        entities,
        turn: Some(TurnFact {
            active: Some(id(base)),
            round: u64::MAX,
        }),
        encounter: EncounterFacts {
            mode: Some(EncounterMode::RealTime),
            pending_mode: Some(Some(EncounterMode::TurnBased)),
            clock: Some(Some(ClockFact {
                round_ticks: u32::MAX,
                elapsed: 2_147_483_647,
            })),
        },
        timeline: Some(
            (0..1024u64)
                .map(|i| TimelineEntry {
                    key: i32::MIN,
                    entity: id(base + i),
                })
                .collect(),
        ),
    }
}

/// The largest body and its 88-frame plan, built once.
fn largest() -> &'static (SnapshotBodyV3, Vec<Vec<u8>>) {
    static CELL: OnceLock<(SnapshotBodyV3, Vec<Vec<u8>>)> = OnceLock::new();
    CELL.get_or_init(|| {
        let body = largest_body();
        let frames = plan(1, 0, 0, 8, &body);
        (body, frames)
    })
}

// ---------------------------------------------------------------------------
// The mirror harness (V-3, V-5): one script on a v3 and a v4 pool.
// ---------------------------------------------------------------------------

struct Mirror {
    v3: ReassemblyPoolV3,
    v4: ReassemblyPoolV4,
    /// `ids[n - 1]` is the pair of receiver id `n`, removed ones included.
    ids: Vec<(ReceiverIdV3, ReceiverIdV4)>,
    /// The pool clock both pools must hold (calls clamp up to it).
    now: u64,
}

impl Mirror {
    fn new(limits: TransferLimitsV3) -> Self {
        let m = Self {
            v3: ReassemblyPoolV3::new(limits.clone()).expect("fixture limits validate"),
            v4: ReassemblyPoolV4::new(limits.clone()).expect("fixture limits validate"),
            ids: vec![],
            now: 0,
        };
        assert_eq!(m.v4.limits(), limits);
        m.check();
        m
    }

    fn pair(&self, rx: u32) -> (ReceiverIdV3, ReceiverIdV4) {
        self.ids[rx as usize - 1]
    }

    /// Both pools' state agrees: every receiver's whole status, both totals.
    fn check(&self) {
        for (a, b) in &self.ids {
            assert_eq!(
                self.v3.status(*a),
                self.v4.status(*b),
                "status of {}",
                a.get()
            );
        }
        assert_eq!(self.v3.staging_bytes(), self.v4.staging_bytes());
        assert_eq!(self.v3.held_delta_bytes(), self.v4.held_delta_bytes());
    }

    /// P-14 on both pools: a refusal that is not `Failed(_)` changes nothing
    /// when the call did not advance the pool clock.
    fn guard<T>(
        &self,
        result: &Result<T, TransferError>,
        quiet: bool,
        before: &(ReassemblyPoolV3, ReassemblyPoolV4),
    ) {
        if quiet && matches!(result, Err(e) if !matches!(e, TransferError::Failed(_))) {
            assert_eq!(self.v3, before.0, "a refusal changed the v3 pool");
            assert_eq!(self.v4, before.1, "a refusal changed the v4 pool");
        }
    }

    fn add(&mut self, mode: GateMode) -> Result<u32, TransferError> {
        let r3 = self.v3.add_receiver(Z16, mode);
        let r4 = self.v4.add_receiver(Z16, mode);
        let out = match (r3, r4) {
            (Ok(a), Ok(b)) => {
                assert_eq!(a.get(), b.get(), "receiver ids differ");
                self.ids.push((a, b));
                Ok(a.get())
            }
            (Err(a), Err(b)) => {
                assert_eq!(a, b);
                Err(a)
            }
            (a, b) => panic!("add_receiver differs: {a:?} against {b:?}"),
        };
        self.check();
        out
    }

    fn remove(&mut self, rx: u32) {
        let (a, b) = self.pair(rx);
        self.v3.remove_receiver(a);
        self.v4.remove_receiver(b);
        self.check();
    }

    fn transfer(
        &mut self,
        rx: u32,
        bytes: &[u8],
        now: u64,
    ) -> Result<TransferOutcome, TransferError> {
        let (a, b) = self.pair(rx);
        let quiet = now <= self.now;
        let before = (self.v3.clone(), self.v4.clone());
        let r3 = self.v3.offer_transfer(a, bytes, now);
        let r4 = self.v4.offer_transfer(b, bytes, now);
        assert_eq!(r3, r4, "offer_transfer differs");
        self.guard(&r3, quiet, &before);
        self.now = self.now.max(now);
        self.check();
        r3
    }

    /// Offers one delta given as its v3 bytes; the v4 pool gets byte 0 = 4.
    fn delta(&mut self, rx: u32, v3_bytes: &[u8], now: u64) -> Result<DeltaOutcome, TransferError> {
        let (a, b) = self.pair(rx);
        let quiet = now <= self.now;
        let before = (self.v3.clone(), self.v4.clone());
        let r3 = self.v3.offer_delta(a, v3_bytes, now);
        let r4 = self.v4.offer_delta(b, &to_v4(v3_bytes), now);
        assert_eq!(r3, r4, "offer_delta differs");
        self.guard(&r3, quiet, &before);
        self.now = self.now.max(now);
        self.check();
        r3
    }

    fn poll(&mut self, now: u64) -> Vec<u32> {
        let r3: Vec<u32> = self.v3.poll(now).into_iter().map(|r| r.get()).collect();
        let r4: Vec<u32> = self.v4.poll(now).into_iter().map(|r| r.get()).collect();
        assert_eq!(r3, r4, "poll differs");
        self.now = self.now.max(now);
        self.check();
        r3
    }

    fn rebind(&mut self, rx: u32, now: u64) -> Result<Rebound, TransferError> {
        let (a, b) = self.pair(rx);
        let quiet = now <= self.now;
        let before = (self.v3.clone(), self.v4.clone());
        let r3 = self.v3.rebind_receiver(a, now);
        let r4 = self.v4.rebind_receiver(b, now);
        assert_eq!(r3, r4, "rebind_receiver differs");
        self.guard(&r3, quiet, &before);
        self.now = self.now.max(now);
        self.check();
        r3
    }

    fn pop(&mut self, rx: u32) -> Option<ReplicaUpdateV3> {
        let (a, b) = self.pair(rx);
        let r3 = self.v3.pop_update(a);
        let r4 = self.v4.pop_update(b);
        assert_eq!(r4, r3.as_ref().map(wrap), "popped update differs");
        self.check();
        r3
    }

    fn status(&self, rx: u32) -> Result<ReceiverStatusV3, TransferError> {
        let (a, b) = self.pair(rx);
        let r3 = self.v3.status(a);
        assert_eq!(r3, self.v4.status(b));
        r3
    }

    fn staging(&self) -> usize {
        assert_eq!(self.v3.staging_bytes(), self.v4.staging_bytes());
        self.v3.staging_bytes()
    }

    fn held(&self) -> usize {
        assert_eq!(self.v3.held_delta_bytes(), self.v4.held_delta_bytes());
        self.v3.held_delta_bytes()
    }
}

fn published(t: u64, released: usize) -> Result<TransferOutcome, TransferError> {
    Ok(TransferOutcome::Published {
        transfer: tid(t),
        released,
    })
}

fn released(count: usize) -> Result<DeltaOutcome, TransferError> {
    Ok(DeltaOutcome::Released { count })
}

fn snapshot(t: u64, base: u64, cursor: u64) -> Option<ReplicaUpdateV3> {
    Some(ReplicaUpdateV3::Snapshot(PublicationV3 {
        transfer: tid(t),
        base_tick: base,
        cursor,
        body: e0(),
    }))
}

fn delta_update(n: u64) -> Option<ReplicaUpdateV3> {
    Some(ReplicaUpdateV3::Delta(dframe(n)))
}

// ---------------------------------------------------------------------------
// V1: the P8 mirror.
// ---------------------------------------------------------------------------

/// The contract's `D'(8)`: `02 00 6C 08 00` in its v3 form.
fn d_prime_8() -> Vec<u8> {
    vec![0x03, 0x00, 0x6C, 0x08, 0x00]
}

/// `O(t, c)`: a Begin at base 10, cursor `c`, 8,192 bytes in 2 chunks.
fn o_begin(t: u64, cursor: u64) -> Vec<u8> {
    begin_frame(BeginTransfer {
        epoch: Z16,
        transfer: tid(t),
        base_tick: 10,
        cursor,
        total_bytes: 8_192,
        entity_count: 0,
        segments_per_chunk: 1,
        chunk_count: 2,
    })
}

/// `O(t)#i` (fill `t`) and `O(t)#0'` (fill `t + 1`).
fn o_chunk(t: u64, index: u8, fill: u8) -> Vec<u8> {
    encode_down(&TransferDown::Chunk(TransferChunk {
        epoch: Z16,
        transfer: tid(t),
        index,
        bytes: vec![fill; 4_096],
    }))
    .expect("fixture chunk encodes")
}

/// `S(t, c)`: `[Begin, Chunk]` of `E0` at base 10, cursor `c`.
fn s_frames(t: u64, cursor: u64, epoch: [u8; 16]) -> Vec<Vec<u8>> {
    let frames = plan_transfer(epoch, tid(t), 10, cursor, 1, &e0()).expect("S plans");
    assert_eq!(frames.len(), 2);
    frames
}

#[test]
fn v1_mirror_matches_the_v3_pool() {
    let mut m = Mirror::new(TransferLimitsV3 {
        deadline_ms: 5_000,
        delta_frames_per_receiver: 6,
        ..TransferLimitsV3::v3()
    });
    let (live, await_snapshot) = (GateMode::Live, GateMode::AwaitSnapshot);

    // 1
    assert_eq!(m.add(live), Ok(1));
    // 2
    assert_eq!(m.delta(1, &d(1), 1000), released(1));
    assert_eq!(m.delta(1, &d(2), 1000), released(1));
    // 3
    assert_eq!(
        m.transfer(1, &o_begin(1, 4), 1000),
        Ok(TransferOutcome::Begun)
    );
    // 4
    assert_eq!(m.delta(1, &d(4), 1000), Ok(DeltaOutcome::Buffered));
    // 5
    assert_eq!(m.delta(1, &d(3), 1000), released(2));
    // 6
    assert_eq!(m.delta(1, &d(5), 1000), Ok(DeltaOutcome::Buffered));
    // 7
    assert_eq!(
        m.transfer(1, &o_begin(1, 4), 1000),
        Ok(TransferOutcome::Duplicate)
    );
    // 8
    assert_eq!(
        m.transfer(1, &o_chunk(1, 0, 1), 1000),
        Ok(TransferOutcome::Stored {
            received: 1,
            chunk_count: 2
        })
    );
    assert_eq!(
        m.transfer(1, &o_chunk(1, 0, 1), 1000),
        Ok(TransferOutcome::Duplicate)
    );
    // 9
    assert_eq!(m.transfer(1, &o_chunk(1, 0, 2), 1000), Err(conflict()));
    // 10
    assert_eq!(
        m.transfer(1, &o_chunk(1, 1, 1), 1000),
        Err(TransferError::StaleTransfer)
    );
    // 11
    assert_eq!(
        m.transfer(1, &o_begin(2, 3), 1000),
        Err(TransferError::BehindReplica)
    );
    // 12
    let s3 = s_frames(3, 5, Z16);
    assert_eq!(m.transfer(1, &s3[0], 1000), Ok(TransferOutcome::Begun));
    // 13
    assert_eq!(m.delta(1, &d(6), 1000), Ok(DeltaOutcome::Buffered));
    // 14
    assert_eq!(m.transfer(1, &s3[1], 1000), published(3, 1));
    // 15
    assert_eq!(m.delta(1, &d(5), 1000), Ok(DeltaOutcome::Superseded));
    // 16
    assert_eq!(m.pop(1), delta_update(1));
    assert_eq!(m.pop(1), delta_update(2));
    assert_eq!(m.pop(1), delta_update(3));
    assert_eq!(m.pop(1), delta_update(4));
    assert_eq!(m.pop(1), snapshot(3, 10, 5));
    assert_eq!(m.pop(1), delta_update(6));
    assert_eq!(m.pop(1), None);
    // 17
    assert_eq!(
        m.transfer(1, &cancel_for(Z16, 9), 1000),
        Ok(TransferOutcome::Cancelled { discarded: false })
    );
    assert_eq!(
        m.transfer(1, &s_frames(9, 6, Z16)[0], 1000),
        Err(TransferError::StaleTransfer)
    );
    // 18
    assert_eq!(
        m.transfer(1, &o_begin(10, 6), 2000),
        Ok(TransferOutcome::Begun)
    );
    // 19
    assert_eq!(m.add(live), Ok(2));
    // 20
    assert_eq!(m.delta(2, &d(1), 6999), released(1));
    // 21
    assert_eq!(m.delta(2, &d(2), 7000), released(1));
    assert_eq!(
        m.status(1),
        Ok(ReceiverStatusV3 {
            mode: GateMode::AwaitSnapshot,
            in_flight: None,
            retired_through: 10,
            published: Some(PublishedBase {
                transfer: tid(3),
                base_tick: 10,
                cursor: 5
            }),
            next_event_seq: 7,
            resync: Some(ResyncReason::Expired),
            ..fresh(live)
        })
    );
    // 22
    assert_eq!(
        m.transfer(1, &o_chunk(10, 0, 10), 7000),
        Err(TransferError::StaleTransfer)
    );
    // 23
    assert_eq!(
        m.transfer(1, &o_begin(11, 6), 8000),
        Ok(TransferOutcome::Begun)
    );
    // 24
    assert_eq!(
        m.transfer(1, &o_chunk(11, 0, 11), 13000),
        Err(TransferError::Failed(FailReason::Expired))
    );
    // 25
    assert_eq!(
        m.transfer(1, &o_begin(12, 6), 14000),
        Ok(TransferOutcome::Begun)
    );
    // 26
    assert_eq!(m.poll(18_999), Vec::<u32>::new());
    assert_eq!(m.poll(19_000), vec![1]);
    assert_eq!(m.poll(100), Vec::<u32>::new());
    // 27
    assert_eq!(
        m.transfer(1, &o_begin(13, 6), 19000),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        m.transfer(1, &cancel_for(Z16, 13), 19000),
        Ok(TransferOutcome::Cancelled { discarded: true })
    );
    assert_eq!(
        m.transfer(1, &cancel_for(Z16, 13), 19000),
        Ok(TransferOutcome::Cancelled { discarded: false })
    );
    // 28
    assert_eq!(m.delta(1, &d(8), 19000), Ok(DeltaOutcome::Buffered));
    assert_eq!(m.delta(1, &d(8), 19000), Ok(DeltaOutcome::Duplicate));
    assert_eq!(m.delta(1, &d_prime_8(), 19000), Err(conflict()));
    // 29
    for n in 9..=13 {
        assert_eq!(
            m.delta(1, &d(n), 19000),
            Ok(DeltaOutcome::Buffered),
            "D({n})"
        );
    }
    assert_eq!(
        m.delta(1, &d(14), 19000),
        Err(TransferError::Failed(FailReason::Overflow))
    );
    // 30
    assert_eq!(m.delta(1, &d(8), 19000), Ok(DeltaOutcome::Buffered));
    assert_eq!(
        m.rebind(1, 19000),
        Ok(Rebound {
            discarded: None,
            dropped_deltas: 1
        })
    );
    // 31
    let s14 = s_frames(14, 7, Z16);
    assert_eq!(m.transfer(1, &s14[0], 19000), Ok(TransferOutcome::Begun));
    assert_eq!(m.transfer(1, &s14[1], 19000), published(14, 0));
    // 32
    assert_eq!(m.delta(1, &d(8), 19000), released(1));
    assert_eq!(m.pop(1), snapshot(14, 10, 7));
    assert_eq!(m.pop(1), delta_update(8));
    assert_eq!(m.pop(1), None);
    // 33
    assert_eq!(
        m.transfer(1, &s_frames(15, 8, [1; 16])[0], 19000),
        Err(TransferError::WrongEpoch)
    );
    // 34
    assert_eq!(m.delta(1, &d(0), 19000), Err(TransferError::DeltaSequence));
    // 35
    assert_eq!(m.add(await_snapshot), Ok(3));
    assert_eq!(m.delta(3, &d(1), 19000), Ok(DeltaOutcome::Buffered));
    // 36: B had 10 queued bytes (D1, D2).
    let (staging, held_before) = (m.staging(), m.held());
    m.remove(2);
    assert_eq!(m.staging(), staging);
    assert_eq!(m.held(), held_before - 10);
    assert_eq!(m.status(2), Err(TransferError::UnknownReceiver));
    assert_eq!(
        m.delta(2, &d(3), 19000),
        Err(TransferError::UnknownReceiver)
    );
}

// ---------------------------------------------------------------------------
// V2: version isolation.
// ---------------------------------------------------------------------------

/// S1 of T026p2 M4: a lane-1 sample, 20 bytes.
fn s1() -> Vec<u8> {
    vec![
        0xA1, 0x01, 0x01, 0x64, 0x01, 0x02, 0x01, 0x80, 0x80, 0x08, 0x00, 0xFF, 0xFF, 0x0F, 0x01,
        0x02, 0x00, 0x00, 0x00, 0x00,
    ]
}

/// T140 D1's Begin: `82 02 10 Z16 01 00 00 06 00 01 01` with a chosen
/// version byte.
fn d1_begin(version: u8) -> Vec<u8> {
    let mut out = vec![version, 0x02, 0x10];
    out.extend_from_slice(&Z16);
    out.extend_from_slice(&[0x01, 0x00, 0x00, 0x06, 0x00, 0x01, 0x01]);
    out
}

const V4_EMPTY: [u8; 5] = [0x04, 0x00, 0x64, 0x01, 0x00];

#[test]
fn v2_version_isolation() {
    // Every call is at clock 0, so no refusal can move the pool's clock (P-14).
    let mut pool = ReassemblyPoolV4::new(l3()).expect("L3 validates");
    let a = pool.add_receiver(Z16, GateMode::Live).expect("admitted");

    // The v1, v2 and v3 encodings of the empty delta with event_seq 1.
    for version in [0x01u8, 0x02, 0x03] {
        let bytes = [version, 0x00, 0x64, 0x01, 0x00];
        let before = pool.clone();
        assert_eq!(
            pool.offer_delta(a, &bytes, 0),
            Err(unsupported()),
            "version {version}"
        );
        assert_eq!(pool, before, "a refusal changed the pool");
    }
    // The v3 encoding really is what the v3 codec writes.
    assert_eq!(
        encode_delta_v3(&DeltaFrameV3 {
            lane: 0,
            server_tick: 100,
            event_seq: 1,
            ops: vec![]
        })
        .expect("encodes"),
        [0x03, 0x00, 0x64, 0x01, 0x00]
    );
    // A 0x81 Begin to the transfer path, a 0xA1 sample and a 0x82 Begin to
    // the delta path, a v4 delta to the transfer path.
    for (bytes, delta_path) in [
        (d1_begin(0x81), false),
        (s1(), true),
        (d1_begin(0x82), true),
        (V4_EMPTY.to_vec(), false),
    ] {
        let before = pool.clone();
        let result = if delta_path {
            pool.offer_delta(a, &bytes, 0).map(|_| ())
        } else {
            pool.offer_transfer(a, &bytes, 0).map(|_| ())
        };
        assert_eq!(result, Err(unsupported()), "{bytes:02X?}");
        assert_eq!(pool, before, "a refusal changed the pool");
    }
    // The S1 bytes to the transfer path as well.
    let before = pool.clone();
    assert_eq!(pool.offer_transfer(a, &s1(), 0), Err(unsupported()));
    assert_eq!(pool, before);

    // Event sequence 0 and an unknown receiver are the pool's, after decode.
    let before = pool.clone();
    assert_eq!(
        pool.offer_delta(a, &[0x04, 0x00, 0x00, 0x00, 0x00], 0),
        Err(TransferError::DeltaSequence)
    );
    assert_eq!(pool, before);

    // Positive control: a v4 frame at the last legal sequence is buffered,
    // and the v4 delta at sequence 1 is released.
    let last = encode_delta_v4(&DeltaFrame {
        lane: 0,
        server_tick: 1,
        event_seq: SEQ_LAST,
        ops: vec![],
    })
    .expect("encodes");
    assert_eq!(pool.offer_delta(a, &last, 0), Ok(DeltaOutcome::Buffered));
    // One past the last legal sequence, and `u64::MAX`, are `DeltaSequence`
    // (the codec reads `event_seq` as a plain `u64`, so the pool refuses).
    for event_seq in [SEQ_LAST + 1, u64::MAX] {
        let bytes = encode_delta_v4(&DeltaFrame {
            lane: 0,
            server_tick: 1,
            event_seq,
            ops: vec![],
        })
        .expect("encodes");
        let before = pool.clone();
        assert_eq!(
            pool.offer_delta(a, &bytes, 0),
            Err(TransferError::DeltaSequence),
            "event_seq {event_seq}"
        );
        assert_eq!(pool, before, "a refusal changed the pool");
    }
    assert_eq!(
        pool.offer_delta(a, &V4_EMPTY, 0),
        Ok(DeltaOutcome::Released { count: 1 })
    );
    assert_eq!(
        pool.pop_update(a),
        Some(ReplicaUpdateV4::Delta(DeltaFrame {
            lane: 0,
            server_tick: 100,
            event_seq: 1,
            ops: vec![],
        }))
    );

    // The v3 pool refuses v4 delta bytes and the lane-1 sample the same way.
    let mut v3 = ReassemblyPoolV3::new(l3()).expect("L3 validates");
    let b = v3.add_receiver(Z16, GateMode::Live).expect("admitted");
    for bytes in [V4_EMPTY.to_vec(), s1()] {
        let before = v3.clone();
        assert_eq!(v3.offer_delta(b, &bytes, 0), Err(unsupported()));
        assert_eq!(v3, before, "a refusal changed the v3 pool");
    }
    // Positive control on the v3 pool: its own bytes are released.
    assert_eq!(
        v3.offer_delta(b, &[0x03, 0x00, 0x64, 0x01, 0x00], 0),
        Ok(DeltaOutcome::Released { count: 1 })
    );
}

// ---------------------------------------------------------------------------
// V3: v4 ops pass the gate unchanged.
// ---------------------------------------------------------------------------

fn frame_f() -> DeltaFrame {
    DeltaFrame {
        lane: 0,
        server_tick: 100,
        event_seq: 1,
        ops: vec![
            DeltaOp::V3(DeltaOpV3::EntityEnter { entity: id(1) }),
            DeltaOp::Moved {
                entity: id(1),
                points: vec![pa(), pb()],
            },
            DeltaOp::Position {
                entity: id(1),
                position: Some(PositionFact {
                    point: pb(),
                    moving: true,
                }),
            },
        ],
    }
}

fn encode_f(frame: &DeltaFrame) -> Vec<u8> {
    encode_delta_v4(frame).expect("fixture frame encodes")
}

#[test]
fn v3_v4_ops_pass_the_gate_unchanged() {
    let f = frame_f();
    let f_bytes = encode_f(&f);
    let len = f_bytes.len();

    // A `Live` receiver releases it and pops it unchanged.
    let mut pool = ReassemblyPoolV4::new(l3()).expect("L3 validates");
    let a = pool.add_receiver(Z16, GateMode::Live).expect("admitted");
    assert_eq!(
        pool.offer_delta(a, &f_bytes, 1_000),
        Ok(DeltaOutcome::Released { count: 1 })
    );
    assert_eq!(pool.held_delta_bytes(), len);
    assert_eq!(
        pool.status(a),
        Ok(ReceiverStatusV3 {
            next_event_seq: 2,
            held_frames: 1,
            held_bytes: len,
            queued_updates: 1,
            ..fresh(GateMode::Live)
        })
    );
    assert_eq!(pool.pop_update(a), Some(ReplicaUpdateV4::Delta(f.clone())));
    assert_eq!(pool.held_delta_bytes(), 0);
    assert_eq!(pool.pop_update(a), None);

    // An `AwaitSnapshot` receiver buffers it; the snapshot at cursor 0
    // publishes, releasing the frame after it.
    let b = pool
        .add_receiver(Z16, GateMode::AwaitSnapshot)
        .expect("admitted");
    assert_eq!(
        pool.offer_delta(b, &f_bytes, 1_000),
        Ok(DeltaOutcome::Buffered)
    );
    assert_eq!(
        pool.status(b),
        Ok(ReceiverStatusV3 {
            held_frames: 1,
            held_bytes: len,
            ..fresh(GateMode::AwaitSnapshot)
        })
    );
    assert_eq!(pool.held_delta_bytes(), len);
    let frames = plan(1, 0, 0, 1, &e0());
    assert_eq!(
        pool.offer_transfer(b, &frames[0], 1_000),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        pool.offer_transfer(b, &frames[1], 1_000),
        Ok(TransferOutcome::Published {
            transfer: tid(1),
            released: 1
        })
    );
    assert_eq!(
        pool.pop_update(b),
        Some(ReplicaUpdateV4::Snapshot(PublicationV3 {
            transfer: tid(1),
            base_tick: 0,
            cursor: 0,
            body: e0(),
        }))
    );
    assert_eq!(pool.pop_update(b), Some(ReplicaUpdateV4::Delta(f)));
    assert_eq!(pool.pop_update(b), None);
    assert_eq!(pool.held_delta_bytes(), 0);
}

#[test]
fn v3_gate_reads_no_op_even_a_dangling_one() {
    // The gate is by `event_seq` alone: ops naming an entity that was never
    // live, a withdrawn position and a 64-point route are released as decoded,
    // in `event_seq` order whatever the arrival order.
    let far = |n: i32| p(n * 65_536, 0, 0);
    let frames = [
        DeltaFrame {
            lane: 0,
            server_tick: 10,
            event_seq: 1,
            ops: vec![DeltaOp::Moved {
                entity: id(9),
                points: vec![pa(), pb()],
            }],
        },
        DeltaFrame {
            lane: 0,
            server_tick: 11,
            event_seq: 2,
            ops: vec![
                DeltaOp::Position {
                    entity: id(9),
                    position: None,
                },
                DeltaOp::Position {
                    entity: id(8),
                    position: Some(PositionFact {
                        point: pa(),
                        moving: false,
                    }),
                },
            ],
        },
        DeltaFrame {
            lane: 0,
            server_tick: 12,
            event_seq: 3,
            ops: vec![DeltaOp::Moved {
                entity: id(7),
                points: (0..64).map(far).collect(),
            }],
        },
    ];
    let bytes: Vec<Vec<u8>> = frames.iter().map(encode_f).collect();
    let mut pool = ReassemblyPoolV4::new(l3()).expect("L3 validates");
    let a = pool.add_receiver(Z16, GateMode::Live).expect("admitted");
    assert_eq!(
        pool.offer_delta(a, &bytes[2], 1_000),
        Ok(DeltaOutcome::Buffered)
    );
    assert_eq!(
        pool.offer_delta(a, &bytes[1], 1_000),
        Ok(DeltaOutcome::Buffered)
    );
    assert_eq!(
        pool.offer_delta(a, &bytes[0], 1_000),
        Ok(DeltaOutcome::Released { count: 3 })
    );
    for frame in &frames {
        assert_eq!(
            pool.pop_update(a),
            Some(ReplicaUpdateV4::Delta(frame.clone()))
        );
    }
    assert_eq!(pool.pop_update(a), None);
    assert_eq!(pool.held_delta_bytes(), 0);
    // A duplicate of a released frame is superseded, a different frame at the
    // same sequence number is not read for its ops either.
    assert_eq!(
        pool.offer_delta(a, &bytes[1], 1_000),
        Ok(DeltaOutcome::Superseded)
    );
}

// ---------------------------------------------------------------------------
// V4: surface and receivers.
// ---------------------------------------------------------------------------

#[test]
fn v4_surface_and_receivers() {
    let limits = l3();
    let mut pool = ReassemblyPoolV4::new(limits.clone()).expect("L3 validates");
    assert_eq!(pool.limits(), limits);
    assert_eq!(pool.staging_bytes(), 0);
    assert_eq!(pool.held_delta_bytes(), 0);

    assert_eq!(
        ReassemblyPoolV4::new(TransferLimitsV3 {
            max_receivers: 0,
            ..limits.clone()
        })
        .map(|_| ()),
        Err(LimitsError::InvalidLimits {
            field: "max_receivers"
        })
    );
    assert_eq!(
        ReassemblyPoolV4::new(TransferLimitsV3 {
            deadline_ms: 20_001,
            ..limits.clone()
        })
        .map(|_| ()),
        Err(LimitsError::LoosensPolicy {
            field: "deadline_ms"
        })
    );
    for bad in [
        TransferLimitsV3 {
            max_receivers: 9,
            ..limits.clone()
        },
        TransferLimitsV3 {
            deadline_ms: 0,
            ..limits.clone()
        },
    ] {
        assert_eq!(
            ReassemblyPoolV4::new(bad.clone()).map(|_| ()),
            ReassemblyPoolV3::new(bad).map(|_| ()),
            "the v4 pool validates as the v3 pool does"
        );
    }

    // Ids are 1, 2; after removing 1 the next is 3.
    let r1 = pool.add_receiver(Z16, GateMode::Live).expect("admitted");
    let r2 = pool
        .add_receiver(Z16, GateMode::AwaitSnapshot)
        .expect("admitted");
    assert_eq!((r1.get(), r2.get()), (1, 2));
    assert!(r1 < r2);
    pool.remove_receiver(r1);
    let r3 = pool.add_receiver(Z16, GateMode::Live).expect("admitted");
    assert_eq!(r3.get(), 3);

    // A fresh receiver's status is P-3's literal, once per mode.
    assert_eq!(pool.status(r2), Ok(fresh(GateMode::AwaitSnapshot)));
    assert_eq!(pool.status(r3), Ok(fresh(GateMode::Live)));
    assert_eq!(pool.status(r1), Err(TransferError::UnknownReceiver));
    let before = pool.clone();
    pool.remove_receiver(r1);
    assert_eq!(pool, before);
    assert_eq!(pool.pop_update(r1), None);

    // Eight receivers fill the pool; the ninth is refused, ids never reused.
    let mut pool = ReassemblyPoolV4::new(l3()).expect("L3 validates");
    let ids: Vec<ReceiverIdV4> = (0..8)
        .map(|_| pool.add_receiver(Z16, GateMode::Live).expect("admitted"))
        .collect();
    assert_eq!(
        ids.iter().map(|r| r.get()).collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5, 6, 7, 8]
    );
    let before = pool.clone();
    assert_eq!(
        pool.add_receiver(Z16, GateMode::Live),
        Err(TransferError::ReceiversFull)
    );
    assert_eq!(pool, before);
    pool.remove_receiver(ids[2]);
    assert_eq!(
        pool.add_receiver(Z16, GateMode::Live)
            .expect("admitted")
            .get(),
        9
    );
    // The epoch a receiver was added with is its status's.
    let mut other_pool = ReassemblyPoolV4::new(l3()).expect("L3 validates");
    let r = other_pool
        .add_receiver([7; 16], GateMode::Live)
        .expect("admitted");
    let other = other_pool.status(r);
    assert_eq!(
        other,
        Ok(ReceiverStatusV3 {
            epoch: [7; 16],
            ..fresh(GateMode::Live)
        })
    );
}

// ---------------------------------------------------------------------------
// V5: the other T140r arms, mirrored on both pools.
// ---------------------------------------------------------------------------

/// P2: the largest body publishes through the pool.
#[test]
fn v5_p2_largest_body_publishes_through_the_pool() {
    let (body, frames) = largest();
    assert_eq!(frames.len(), 88);
    let mut m = Mirror::new(l3());
    assert_eq!(m.add(GateMode::AwaitSnapshot), Ok(1));
    assert_eq!(m.transfer(1, &frames[0], 1_000), Ok(TransferOutcome::Begun));
    for (k, index) in (1..=86usize).rev().enumerate() {
        assert_eq!(
            m.transfer(1, &frames[1 + index], 1_000),
            Ok(TransferOutcome::Stored {
                received: k as u8 + 1,
                chunk_count: 87
            }),
            "chunk {index}"
        );
    }
    assert_eq!(
        m.status(1),
        Ok(ReceiverStatusV3 {
            in_flight: Some(InFlightStatus {
                transfer: tid(1),
                cursor: 0,
                base_tick: 0,
                received: 86,
                chunk_count: 87,
                deadline_ms: 21_000,
                reserved_bytes: 2_847_788,
            }),
            ..fresh(GateMode::AwaitSnapshot)
        })
    );
    assert_eq!(m.staging(), 2_847_788);
    assert_eq!(m.transfer(1, &frames[1], 1_000), published(1, 0));
    assert_eq!(
        m.pop(1),
        Some(ReplicaUpdateV3::Snapshot(PublicationV3 {
            transfer: tid(1),
            base_tick: 0,
            cursor: 0,
            body: body.clone(),
        }))
    );
    assert_eq!(m.pop(1), None);
    assert_eq!(m.staging(), 0);
    assert_eq!(
        m.status(1),
        Ok(ReceiverStatusV3 {
            mode: GateMode::Live,
            retired_through: 1,
            published: Some(PublishedBase {
                transfer: tid(1),
                base_tick: 0,
                cursor: 0
            }),
            ..fresh(GateMode::Live)
        })
    );
}

/// P3: staging, per-receiver and aggregate limits.
#[test]
fn v5_p3_full_envelope_staging_and_aggregate() {
    // Eight maximal transfers fill the pool's 32 MiB.
    let mut m = Mirror::new(l3());
    for n in 1..=8 {
        assert_eq!(m.add(GateMode::AwaitSnapshot), Ok(n));
    }
    for n in 1..=8 {
        assert_eq!(m.transfer(n, &g(1), 1_000), Ok(TransferOutcome::Begun));
    }
    assert_eq!(m.staging(), 33_554_432);

    // The per-receiver bound: exactly the largest body fits, one byte more
    // does not.
    let mut m = Mirror::new(TransferLimitsV3 {
        staging_bytes_per_receiver: 2_847_788,
        ..l3()
    });
    assert_eq!(m.add(GateMode::AwaitSnapshot), Ok(1));
    assert_eq!(m.add(GateMode::AwaitSnapshot), Ok(2));
    assert_eq!(
        m.transfer(1, &b_frame(1, 2_847_788), 1_000),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        m.transfer(2, &b_frame(1, 2_847_789), 1_000),
        Err(TransferError::StagingFull)
    );
    assert_eq!(m.staging(), 2_847_788);

    // The aggregate bound, and room returned by a cancel and a removal.
    let mut m = Mirror::new(TransferLimitsV3 {
        staging_bytes_total: 7_042_092,
        ..l3()
    });
    assert_eq!(m.add(GateMode::AwaitSnapshot), Ok(1));
    assert_eq!(m.add(GateMode::AwaitSnapshot), Ok(2));
    assert_eq!(m.transfer(1, &g(1), 1_000), Ok(TransferOutcome::Begun));
    assert_eq!(
        m.transfer(2, &b_frame(1, 2_847_789), 1_000),
        Err(TransferError::AggregateFull)
    );
    assert_eq!(
        m.transfer(2, &b_frame(1, 2_847_788), 1_000),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(m.staging(), 7_042_092);
    assert_eq!(
        m.transfer(1, &cancel_for(Z16, 1), 1_000),
        Ok(TransferOutcome::Cancelled { discarded: true })
    );
    assert_eq!(m.staging(), 2_847_788);
    m.remove(2);
    assert_eq!(m.staging(), 0);
}

/// P4: the 19,999 / 20,000 ms deadline boundary.
#[test]
fn v5_p4_deadline_boundary_19999_20000() {
    let (_, frames) = largest();
    let drive = |m: &mut Mirror| {
        assert_eq!(m.add(GateMode::AwaitSnapshot), Ok(1));
        assert_eq!(m.transfer(1, &frames[0], 1_000), Ok(TransferOutcome::Begun));
        for index in (1..=86usize).rev() {
            assert!(matches!(
                m.transfer(1, &frames[1 + index], 1_000),
                Ok(TransferOutcome::Stored { .. })
            ));
        }
    };

    // Pool 1: chunk 0 at 20,999 is the last live moment.
    let mut m = Mirror::new(l3());
    drive(&mut m);
    assert_eq!(m.transfer(1, &frames[1], 20_999), published(1, 0));
    assert_eq!(m.staging(), 0);

    // Pool 2: chunk 0 at 21,000 is too late.
    let mut m = Mirror::new(l3());
    drive(&mut m);
    assert_eq!(
        m.transfer(1, &frames[1], 21_000),
        Err(TransferError::Failed(FailReason::Expired))
    );
    assert_eq!(
        m.status(1),
        Ok(ReceiverStatusV3 {
            retired_through: 1,
            resync: Some(ResyncReason::Expired),
            ..fresh(GateMode::AwaitSnapshot)
        })
    );
    assert_eq!(m.staging(), 0);
    assert_eq!(
        m.transfer(1, &frames[1], 21_000),
        Err(TransferError::StaleTransfer)
    );
    // A call at an earlier clock is clamped, and refuses without a change.
    assert_eq!(
        m.transfer(1, &frames[2], 10),
        Err(TransferError::StaleTransfer)
    );

    // Pool 3: poll reports the expiry exactly at 20,000 ms after the Begin.
    let mut m = Mirror::new(l3());
    assert_eq!(m.add(GateMode::AwaitSnapshot), Ok(1));
    assert_eq!(m.transfer(1, &frames[0], 1_000), Ok(TransferOutcome::Begun));
    assert_eq!(m.poll(20_999), Vec::<u32>::new());
    assert_eq!(m.poll(21_000), vec![1]);
    assert_eq!(m.staging(), 0);

    // Pool 4: the deadline saturates.
    let mut m = Mirror::new(l3());
    assert_eq!(m.add(GateMode::AwaitSnapshot), Ok(1));
    let tiny = plan(1, 0, 0, 1, &e0());
    assert_eq!(
        m.transfer(1, &tiny[0], u64::MAX - 10),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        m.status(1).expect("receiver exists").in_flight,
        Some(InFlightStatus {
            transfer: tid(1),
            cursor: 0,
            base_tick: 0,
            received: 0,
            chunk_count: 1,
            deadline_ms: u64::MAX,
            reserved_bytes: 6,
        })
    );
}

/// P5: invalid bodies never publish.
#[test]
fn v5_p5_invalid_bodies_never_publish() {
    let mut m = Mirror::new(l3());
    assert_eq!(m.add(GateMode::AwaitSnapshot), Ok(1));
    let first = plan(1, 0, 0, 1, &e0());
    assert_eq!(m.transfer(1, &first[0], 1_000), Ok(TransferOutcome::Begun));
    assert_eq!(m.transfer(1, &first[1], 1_000), published(1, 0));

    let published_base = Some(PublishedBase {
        transfer: tid(1),
        base_tick: 0,
        cursor: 0,
    });
    let after = |retired: u64| ReceiverStatusV3 {
        retired_through: retired,
        published: published_base.clone(),
        resync: Some(ResyncReason::Invalid),
        queued_updates: 1,
        ..fresh(GateMode::AwaitSnapshot)
    };

    // G(2): 4 MiB whose first six bytes decode as `E0` with trailing bytes.
    assert_eq!(m.transfer(1, &g(2), 1_000), Ok(TransferOutcome::Begun));
    for i in 0..=126u8 {
        assert_eq!(
            m.transfer(1, &g_chunk(2, i), 1_000),
            Ok(TransferOutcome::Stored {
                received: i + 1,
                chunk_count: 128
            }),
            "chunk {i}"
        );
    }
    assert_eq!(
        m.transfer(1, &g_chunk(2, 127), 1_000),
        Err(TransferError::Failed(FailReason::InvalidBody(
            CodecError::Malformed
        )))
    );
    assert_eq!(m.staging(), 0);
    assert_eq!(m.status(1), Ok(after(2)));

    // Transfer 3: an entity count of 1 against a Begin that says 0.
    let small = |t: u64, total: u32| {
        begin_frame(BeginTransfer {
            epoch: Z16,
            transfer: tid(t),
            base_tick: 0,
            cursor: 0,
            total_bytes: total,
            entity_count: 0,
            segments_per_chunk: 1,
            chunk_count: 1,
        })
    };
    let chunk = |t: u64, bytes: Vec<u8>| {
        encode_down(&TransferDown::Chunk(TransferChunk {
            epoch: Z16,
            transfer: tid(t),
            index: 0,
            bytes,
        }))
        .expect("fixture chunk encodes")
    };
    let one_entity = vec![
        0x01, 0x01, 0x01, 0x0A, 0x0A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00,
    ];
    assert_eq!(one_entity.len(), 16);
    assert_eq!(
        m.transfer(1, &small(3, 16), 1_000),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        m.transfer(1, &chunk(3, one_entity), 1_000),
        Err(TransferError::Failed(FailReason::InvalidBody(
            CodecError::Malformed
        )))
    );
    assert_eq!(m.staging(), 0);
    assert_eq!(m.status(1), Ok(after(3)));

    // Transfer 4: `E0` plus one trailing byte.
    assert_eq!(
        m.transfer(1, &small(4, 7), 1_000),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        m.transfer(1, &chunk(4, vec![0; 7]), 1_000),
        Err(TransferError::Failed(FailReason::InvalidBody(
            CodecError::Malformed
        )))
    );
    assert_eq!(m.staging(), 0);
    assert_eq!(m.status(1), Ok(after(4)));

    // Only transfer 1's snapshot ever reached the queue.
    assert_eq!(m.pop(1), snapshot(1, 0, 0));
    assert_eq!(m.pop(1), None);
}

/// P9: rebind discards the in-flight transfer and the held deltas.
#[test]
fn v5_p9_rebind_discards_in_flight_and_held() {
    let mut m = Mirror::new(l3());
    assert_eq!(m.add(GateMode::Live), Ok(1));
    assert_eq!(m.delta(1, &d(1), 1_000), released(1));
    // Cursor 0 would be BehindReplica now: next_event_seq is 2.
    assert_eq!(
        m.transfer(1, &g(1), 1_000),
        Err(TransferError::BehindReplica)
    );
    assert_eq!(
        m.transfer(1, &g_at(1, 1), 1_000),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(m.delta(1, &d(2), 1_000), Ok(DeltaOutcome::Buffered));
    assert_eq!(m.delta(1, &d(3), 1_000), Ok(DeltaOutcome::Buffered));
    assert_eq!(
        m.rebind(1, 1_000),
        Ok(Rebound {
            discarded: Some(tid(1)),
            dropped_deltas: 2
        })
    );
    assert_eq!(
        m.status(1),
        Ok(ReceiverStatusV3 {
            mode: GateMode::AwaitSnapshot,
            retired_through: 1,
            next_event_seq: 2,
            held_frames: 1,
            held_bytes: 5,
            queued_updates: 1,
            ..fresh(GateMode::Live)
        })
    );
    assert_eq!(m.staging(), 0);
    assert_eq!(m.held(), 5);
    assert_eq!(m.pop(1), delta_update(1));
    assert_eq!(m.pop(1), None);
    m.remove(1);
    assert_eq!(m.rebind(1, 1_000), Err(TransferError::UnknownReceiver));
}

/// P10: the Begin, Chunk and Cancel arms.
fn ob(t: u64, base: u64, cursor: u64) -> Vec<u8> {
    begin_frame(BeginTransfer {
        epoch: Z16,
        transfer: tid(t),
        base_tick: base,
        cursor,
        total_bytes: 8_192,
        entity_count: 0,
        segments_per_chunk: 1,
        chunk_count: 2,
    })
}

fn oc(t: u64, index: u8, len: usize, epoch: [u8; 16]) -> Vec<u8> {
    encode_down(&TransferDown::Chunk(TransferChunk {
        epoch,
        transfer: tid(t),
        index,
        bytes: vec![t as u8; len],
    }))
    .expect("fixture chunk encodes")
}

#[test]
fn v5_p10_begin_chunk_and_cancel_arms() {
    let mut m = Mirror::new(l3());
    assert_eq!(m.add(GateMode::Live), Ok(1));
    let failed = |retired: u64| ReceiverStatusV3 {
        mode: GateMode::AwaitSnapshot,
        retired_through: retired,
        published: Some(PublishedBase {
            transfer: tid(1),
            base_tick: 20,
            cursor: 0,
        }),
        queued_updates: 1,
        resync: Some(ResyncReason::Invalid),
        ..fresh(GateMode::Live)
    };

    // 1
    let one = plan(1, 20, 0, 1, &e0());
    assert_eq!(m.transfer(1, &one[0], 1_000), Ok(TransferOutcome::Begun));
    assert_eq!(m.transfer(1, &one[1], 1_000), published(1, 0));
    // 2
    assert_eq!(
        m.transfer(1, &ob(2, 20, 0), 1_000),
        Ok(TransferOutcome::Begun)
    );
    // 3
    assert_eq!(
        m.transfer(1, &ob(3, 20, 0), 1_000),
        Err(TransferError::InFlight)
    );
    // 4
    assert_eq!(
        m.transfer(1, &oc(5, 0, 4_096, Z16), 1_000),
        Err(TransferError::UnknownTransfer)
    );
    // 5
    assert_eq!(
        m.transfer(1, &oc(2, 0, 4_096, [1; 16]), 1_000),
        Err(TransferError::WrongEpoch)
    );
    assert_eq!(
        m.transfer(1, &cancel_for([1; 16], 2), 1_000),
        Err(TransferError::WrongEpoch)
    );
    // 6
    assert_eq!(
        m.transfer(1, &cancel_for(Z16, 1), 1_000),
        Ok(TransferOutcome::Cancelled { discarded: false })
    );
    let status = m.status(1).expect("receiver exists");
    assert_eq!(status.in_flight.map(|f| f.transfer), Some(tid(2)));
    assert_eq!(status.retired_through, 1);
    // 7
    assert_eq!(m.transfer(1, &ob(2, 20, 1), 1_000), Err(conflict()));
    assert_eq!(m.staging(), 0);
    assert_eq!(m.status(1), Ok(failed(2)));
    // 8
    assert_eq!(
        m.transfer(1, &ob(3, 19, 0), 1_000),
        Err(TransferError::BehindReplica)
    );
    assert_eq!(
        m.transfer(1, &ob(3, 20, 0), 1_000),
        Ok(TransferOutcome::Begun)
    );
    // 9
    assert_eq!(m.transfer(1, &oc(3, 2, 4_096, Z16), 1_000), Err(conflict()));
    assert_eq!(m.staging(), 0);
    assert_eq!(m.status(1), Ok(failed(3)));
    // 10
    assert_eq!(
        m.transfer(1, &ob(4, 20, 0), 1_000),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(m.transfer(1, &oc(4, 0, 4_095, Z16), 1_000), Err(conflict()));
    assert_eq!(m.staging(), 0);
    assert_eq!(m.status(1), Ok(failed(4)));
    // Transfer 1's snapshot was never withdrawn.
    assert_eq!(m.pop(1), snapshot(1, 20, 0));
    assert_eq!(m.pop(1), None);
}

/// P11's `TL(n, k)`: a v3 frame holding one `Timeline` of `k` entries.
fn tl(n: u64, k: u64) -> Vec<u8> {
    assert!(n <= 127);
    encode_delta_v3(&DeltaFrameV3 {
        lane: 0,
        server_tick: n,
        event_seq: n,
        ops: vec![DeltaOpV3::Timeline {
            entries: Some(
                (1..=k)
                    .map(|i| TimelineEntry {
                        key: 0,
                        entity: id(i),
                    })
                    .collect(),
            ),
        }],
    })
    .expect("fixture frame encodes")
}

#[test]
fn v5_p11_queue_clear_overflow_bytes_and_poll_order() {
    assert_eq!(tl(1, 1_024).len(), 2_954);
    assert_eq!(tl(1, 222).len(), 548);

    // (a) Queue clear: a publication over a queued snapshot empties the queue.
    let mut m = Mirror::new(l3());
    assert_eq!(m.add(GateMode::Live), Ok(1));
    let one = plan(1, 0, 0, 1, &e0());
    assert_eq!(m.transfer(1, &one[0], 1_000), Ok(TransferOutcome::Begun));
    assert_eq!(m.transfer(1, &one[1], 1_000), published(1, 0));
    assert_eq!(m.delta(1, &d(1), 1_000), released(1));
    let two = plan(2, 0, 1, 1, &e0());
    assert_eq!(m.transfer(1, &two[0], 1_000), Ok(TransferOutcome::Begun));
    assert_eq!(m.transfer(1, &two[1], 1_000), published(2, 0));
    assert_eq!(m.status(1).expect("receiver exists").queued_updates, 1);
    assert_eq!(m.held(), 0);
    assert_eq!(m.pop(1), snapshot(2, 0, 1));
    assert_eq!(m.pop(1), None);

    // (b) The per-receiver byte budget, exactly.
    let mut m = Mirror::new(TransferLimitsV3 {
        delta_bytes_per_receiver: 65_536,
        ..l3()
    });
    assert_eq!(m.add(GateMode::AwaitSnapshot), Ok(1));
    for n in 1..=22 {
        assert_eq!(
            m.delta(1, &tl(n, 1_024), 1_000),
            Ok(DeltaOutcome::Buffered),
            "TL({n})"
        );
    }
    assert_eq!(m.delta(1, &tl(23, 222), 1_000), Ok(DeltaOutcome::Buffered));
    assert_eq!(m.status(1).expect("receiver exists").held_bytes, 65_536);
    assert_eq!(
        m.delta(1, &d(24), 1_000),
        Err(TransferError::Failed(FailReason::Overflow))
    );
    assert_eq!(
        m.status(1),
        Ok(ReceiverStatusV3 {
            resync: Some(ResyncReason::Overflow),
            ..fresh(GateMode::AwaitSnapshot)
        })
    );
    assert_eq!(m.held(), 0);

    // (c) The pool byte budget, exactly.
    let mut m = Mirror::new(TransferLimitsV3 {
        delta_bytes_per_receiver: 65_536,
        delta_bytes_total: 65_536,
        ..l3()
    });
    assert_eq!(m.add(GateMode::AwaitSnapshot), Ok(1));
    assert_eq!(m.add(GateMode::AwaitSnapshot), Ok(2));
    for n in 1..=22 {
        assert_eq!(m.delta(1, &tl(n, 1_024), 1_000), Ok(DeltaOutcome::Buffered));
    }
    assert_eq!(m.delta(2, &tl(1, 222), 1_000), Ok(DeltaOutcome::Buffered));
    assert_eq!(m.held(), 65_536);
    assert_eq!(
        m.delta(2, &d(2), 1_000),
        Err(TransferError::Failed(FailReason::Overflow))
    );
    assert_eq!(m.held(), 64_988);
    let a = m.status(1).expect("receiver exists");
    assert_eq!((a.held_frames, a.held_bytes), (22, 64_988));

    // (d) Queued updates survive Overflow.
    let mut m = Mirror::new(TransferLimitsV3 {
        delta_frames_per_receiver: 2,
        ..l3()
    });
    assert_eq!(m.add(GateMode::Live), Ok(1));
    assert_eq!(m.delta(1, &d(1), 1_000), released(1));
    assert_eq!(m.delta(1, &d(3), 1_000), Ok(DeltaOutcome::Buffered));
    assert_eq!(
        m.delta(1, &d(4), 1_000),
        Err(TransferError::Failed(FailReason::Overflow))
    );
    assert_eq!(
        m.status(1),
        Ok(ReceiverStatusV3 {
            mode: GateMode::AwaitSnapshot,
            next_event_seq: 2,
            held_frames: 1,
            held_bytes: 5,
            queued_updates: 1,
            resync: Some(ResyncReason::Overflow),
            ..fresh(GateMode::Live)
        })
    );
    assert_eq!(m.pop(1), delta_update(1));
    assert_eq!(m.pop(1), None);

    // (e) Poll order is ascending whatever the order the transfers began.
    let mut m = Mirror::new(l3());
    assert_eq!(m.add(GateMode::AwaitSnapshot), Ok(1));
    assert_eq!(m.add(GateMode::AwaitSnapshot), Ok(2));
    assert_eq!(m.transfer(2, &g(1), 1_000), Ok(TransferOutcome::Begun));
    assert_eq!(m.transfer(1, &g(1), 1_000), Ok(TransferOutcome::Begun));
    assert_eq!(m.poll(21_000), vec![1, 2]);
    assert_eq!(m.staging(), 0);
}

// ---------------------------------------------------------------------------
// The harness itself must be able to tell the pools apart.
// ---------------------------------------------------------------------------

#[test]
fn harness_notices_a_divergent_pool() {
    // If the v4 pool differed, the comparison inside `Mirror` would fire; show
    // that a status difference between two live pools is visible to it.
    let mut m = Mirror::new(l3());
    assert_eq!(m.add(GateMode::Live), Ok(1));
    let (_, b) = m.pair(1);
    // Feed only the v4 pool one frame: the whole-status comparison fails.
    m.v4.offer_delta(b, &to_v4(&d(1)), 1_000).expect("accepted");
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| m.check()));
    assert!(outcome.is_err(), "a divergence went unnoticed");
}
