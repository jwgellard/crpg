//! T049a: the smoother's clock, buffers and render (S1–S8), the v4 client's
//! feed points (C1–C3), and the walk session's drawn positions (E1–E2).
//!
//! Every expected number is the contract's, never a value read from the
//! implementation. The walk cases drive the real-time v4 session on the run
//! loop with `authority::v4::rt_tests`' manual clock and rendezvous helpers.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crpg_core::Ulid;
use crpg_net::movement_v4::{encode_down, MoveDown, MoveSample, SampleEntry};
use crpg_net::protocol::{NetId, SessionEpoch};
use crpg_net::protocol_v3::DeltaOp as DeltaOpV3;
use crpg_net::protocol_v4::{DeltaOp, Point};
use crpg_net::replica_v4::PositionState;
use crpg_net::transport::Transport;

use super::{Sample, Smoother, RENDER_DELAY_MS, SAMPLES_PER_ENTITY, TICK_MS};
use crate::core::authority::v4::rt_tests::{settle, walk_root, ManualClock};
use crate::core::client::{PresentationRow, PresentationTable};
use crate::core::client_v4::ClientV4;
use crate::core::errors::BridgeCode;
use crate::core::link::{LocalLink, LINK_FRAMES};
use crate::core::{BridgeCore, Probe};

// ---------------------------------------------------------------------------
// Shared helpers.
// ---------------------------------------------------------------------------

/// `NetId::new(k).unwrap()`.
fn n(k: u64) -> NetId {
    NetId::new(k).expect("a nonzero id")
}

/// A disclosed position at `tick`.
fn st(tick: u64, point: [i32; 3], moving: bool) -> PositionState {
    PositionState {
        tick,
        point: Point::new(point).expect("in range"),
        moving,
    }
}

/// A disclosed-position map from `(id, state)` pairs.
fn map<const N: usize>(entries: [(u64, PositionState); N]) -> BTreeMap<NetId, PositionState> {
    entries
        .into_iter()
        .map(|(entity, state)| (n(entity), state))
        .collect()
}

/// A one-entity disclosed-position map.
fn one(entity: u64, state: PositionState) -> BTreeMap<NetId, PositionState> {
    map([(entity, state)])
}

// ---------------------------------------------------------------------------
// S1.
// ---------------------------------------------------------------------------

/// B-S3 (1): without a clock the newest point is held at every time.
#[test]
fn hold_before_any_clock() {
    let mut smoother = Smoother::new();
    smoother.observe(&one(1, st(5, [1000, 0, 0], false)));
    assert_eq!(smoother.render(n(1), 0), Some([1000, 0, 0]));
    assert_eq!(smoother.render(n(1), 99_999), Some([1000, 0, 0]));
    assert_eq!(smoother.render(n(9), 0), None);
}

// ---------------------------------------------------------------------------
// S2.
// ---------------------------------------------------------------------------

/// B-S1, B-S3: the delay, the offset, the hold and the before-the-oldest rule.
#[test]
fn lerp_between_moving_samples() {
    let mut smoother = Smoother::new();
    smoother.observe_clock(10, 600);
    smoother.observe(&one(1, st(10, [0, 0, 0], true)));
    smoother.observe(&one(1, st(11, [6554, 0, 0], true)));

    assert_eq!(smoother.render(n(1), 650), Some([0, 0, 0]));
    assert_eq!(smoother.render(n(1), 700), Some([0, 0, 0]));
    assert_eq!(smoother.render(n(1), 725), Some([3277, 0, 0]));
    assert_eq!(smoother.render(n(1), 749), Some([6422, 0, 0]));
    assert_eq!(smoother.render(n(1), 750), Some([6554, 0, 0]));
    assert_eq!(smoother.render(n(1), 900), Some([6554, 0, 0]));
}

// ---------------------------------------------------------------------------
// S3.
// ---------------------------------------------------------------------------

/// B-S3 (4): the division truncates toward zero on a decreasing coordinate.
#[test]
fn division_truncates_toward_zero() {
    let mut smoother = Smoother::new();
    smoother.observe_clock(10, 600);
    smoother.observe(&one(1, st(10, [6554, 0, 0], true)));
    smoother.observe(&one(1, st(11, [0, 0, 0], true)));

    assert_eq!(smoother.render(n(1), 749), Some([132, 0, 0]));
    assert_eq!(smoother.render(n(1), 650), Some([6554, 0, 0]));
}

// ---------------------------------------------------------------------------
// S4.
// ---------------------------------------------------------------------------

/// B-S3 (4), Reading 3: a rest followed by a move creeps over one tick only.
#[test]
fn rest_then_move_uses_the_last_tick() {
    let mut smoother = Smoother::new();
    smoother.observe_clock(10, 600);
    smoother.observe(&one(1, st(10, [0, 0, 0], false)));
    smoother.observe(&one(1, st(14, [6554, 0, 0], true)));

    assert_eq!(smoother.render(n(1), 800), Some([0, 0, 0]));
    assert_eq!(smoother.render(n(1), 875), Some([3277, 0, 0]));
    assert_eq!(smoother.render(n(1), 900), Some([6554, 0, 0]));

    // Control: a moving earlier sample uses the whole gap.
    let mut control = Smoother::new();
    control.observe_clock(10, 600);
    control.observe(&one(1, st(10, [0, 0, 0], true)));
    control.observe(&one(1, st(14, [6554, 0, 0], true)));
    assert_eq!(control.render(n(1), 800), Some([3277, 0, 0]));
}

// ---------------------------------------------------------------------------
// S5.
// ---------------------------------------------------------------------------

/// B-S1: the offset is the minimum over every sample, never raised later.
#[test]
fn offset_is_the_minimum() {
    let mut smoother = Smoother::new();
    smoother.observe_clock(10, 600);
    smoother.observe_clock(11, 640);
    smoother.observe_clock(9, 2000);
    smoother.observe_clock(12, 700);
    assert_eq!(smoother.offset_ms(), Some(90));
}

// ---------------------------------------------------------------------------
// S6.
// ---------------------------------------------------------------------------

/// B-S2: the bound, the replace, the restart and the revocation rule.
#[test]
fn buffers_follow_the_replica() {
    let mut smoother = Smoother::new();
    for k in 1..=9u64 {
        smoother.observe(&one(1, st(k, [100 * k as i32, 0, 0], true)));
    }
    let ticks: Vec<u64> = smoother
        .buffer(n(1))
        .expect("a buffer")
        .iter()
        .map(|sample| sample.tick)
        .collect();
    assert_eq!(ticks, (2..=9).collect::<Vec<u64>>());

    // The same tick with a new value replaces the newest; the length holds.
    smoother.observe(&one(1, st(9, [5, 5, 5], false)));
    let buffer = smoother.buffer(n(1)).expect("a buffer");
    assert_eq!(buffer.len(), SAMPLES_PER_ENTITY);
    let ticks: Vec<u64> = buffer.iter().map(|sample| sample.tick).collect();
    assert_eq!(ticks, (2..=9).collect::<Vec<u64>>());
    assert_eq!(
        buffer.back(),
        Some(&Sample {
            tick: 9,
            point: [5, 5, 5],
            moving: false,
        })
    );

    // The identical sample changes nothing.
    let before = smoother.buffer(n(1)).cloned();
    smoother.observe(&one(1, st(9, [5, 5, 5], false)));
    assert_eq!(smoother.buffer(n(1)).cloned(), before);

    // An older tick restarts the buffer.
    smoother.observe(&one(1, st(3, [7, 0, 0], false)));
    assert_eq!(
        smoother.buffer(n(1)),
        Some(&VecDeque::from([Sample {
            tick: 3,
            point: [7, 0, 0],
            moving: false,
        }]))
    );

    // An id with no disclosed position is dropped at once.
    smoother.observe(&map([
        (1, st(4, [7, 0, 0], false)),
        (2, st(1, [0, 0, 0], false)),
    ]));
    smoother.observe(&one(2, st(1, [0, 0, 0], false)));
    assert_eq!(smoother.buffer(n(1)), None);
    assert_eq!(smoother.render(n(1), 0), None);
    assert!(smoother.buffer(n(2)).is_some());
}

// ---------------------------------------------------------------------------
// S7.
// ---------------------------------------------------------------------------

/// B-S3, Reading 5: the time and interpolation arithmetic is `i128`.
#[test]
fn no_overflow_at_the_limits() {
    let mut smoother = Smoother::new();
    smoother.observe_clock(0, 0);
    smoother.observe(&one(1, st(0, [-536_870_912, 0, 0], true)));
    smoother.observe(&one(1, st(1 << 40, [536_870_912, 0, 0], true)));
    assert_eq!(smoother.render(n(1), (1 << 40) * 25 + 100), Some([0, 0, 0]));

    let mut fresh = Smoother::new();
    fresh.observe(&one(1, st(u64::MAX, [1, 2, 3], false)));
    fresh.observe_clock(u64::MAX, 0);
    assert_eq!(fresh.render(n(1), 0), Some([1, 2, 3]));
}

// ---------------------------------------------------------------------------
// S8.
// ---------------------------------------------------------------------------

/// B-S3's constants agree with the server and ADR-0036 U7.
#[test]
fn constants_match_the_server_and_the_adr() {
    assert_eq!(TICK_MS, crpg_server::run::TICK_PERIOD_MS);
    assert_eq!(RENDER_DELAY_MS, 2 * TICK_MS);
    assert_eq!(SAMPLES_PER_ENTITY, 8);
}

// ---------------------------------------------------------------------------
// C1–C3 helpers, copied from `client_v4/tests.rs` (the originals are private).
// ---------------------------------------------------------------------------

/// The v4 test epoch.
fn epoch() -> SessionEpoch {
    SessionEpoch([1; 16])
}

/// A nonzero replica id.
fn id(raw: u64) -> NetId {
    NetId::new(raw).expect("nonzero id")
}

/// A two-row presentation table.
fn table() -> PresentationTable {
    let mut rows = std::collections::BTreeMap::new();
    rows.insert(
        id(1),
        PresentationRow {
            placement: Ulid::from_u128(14),
            name: "Hero".to_string(),
            side: "Player".to_string(),
        },
    );
    rows.insert(
        id(2),
        PresentationRow {
            placement: Ulid::from_u128(15),
            name: "Goblin".to_string(),
            side: "Monster".to_string(),
        },
    );
    let mut abilities = std::collections::BTreeMap::new();
    abilities.insert(Ulid::from_u128(10), "Strike".to_string());
    PresentationTable {
        rows,
        abilities,
        movement_pool: None,
    }
}

/// A live v4 client half.
fn live() -> ClientV4 {
    ClientV4::live_for_test(epoch(), table())
}

/// A point from raw coordinates.
fn p(x: i32, y: i32, z: i32) -> Point {
    Point::new([x, y, z]).expect("in range")
}

/// One lane-0 delta frame.
fn v4(seq: u64, tick: u64, ops: Vec<DeltaOp>) -> Vec<u8> {
    crpg_net::codec_v4::encode_delta(&crpg_net::protocol_v4::DeltaFrame {
        lane: 0,
        server_tick: tick,
        event_seq: seq,
        ops,
    })
    .expect("frame encodes")
}

/// An entity-enter op.
fn enter(entity: u64) -> DeltaOp {
    DeltaOp::V3(DeltaOpV3::EntityEnter { entity: id(entity) })
}

/// A position-anchor op.
fn pos(entity: u64, point: Point, moving: bool) -> DeltaOp {
    DeltaOp::Position {
        entity: id(entity),
        position: Some(crpg_net::protocol_v4::PositionFact { point, moving }),
    }
}

/// Queues one frame on the authority end of `link`.
fn push(link: &LocalLink, bytes: Vec<u8>) {
    link.authority_end().send(&bytes).expect("link has room");
}

/// One lane-1 sample frame.
fn sample(tick: u64, entries: Vec<(u64, Point, bool)>) -> Vec<u8> {
    encode_down(&MoveDown::Sample(MoveSample {
        server_tick: tick,
        processed_seq: 0,
        entries: entries
            .into_iter()
            .map(|(entity, point, moving)| SampleEntry {
                entity: id(entity),
                point,
                moving,
            })
            .collect(),
    }))
    .expect("sample encodes")
}

// ---------------------------------------------------------------------------
// C1.
// ---------------------------------------------------------------------------

/// B-S4: every sample applied inside a step is buffered and clocks the render.
#[test]
fn client_feeds_every_sample_it_applies() {
    let link = LocalLink::new(LINK_FRAMES);
    let mut end = link.client_end();
    let mut client = live();

    push(
        &link,
        v4(1, 5, vec![enter(1), enter(2), pos(1, p(0, 0, 0), true)]),
    );
    client.step(&mut end, &[], false, 0).expect("first frame");
    assert_eq!(client.render_positions(0), vec![0, 0, 0, 0, 0, 0]);

    push(&link, sample(6, vec![(1, p(6554, 0, 0), true)]));
    client
        .step(&mut end, &[], false, 400)
        .expect("sample frame");
    assert_eq!(client.render_positions(475), vec![3277, 0, 0, 0, 0, 0]);
    assert_eq!(client.view().positions(), &[6554, 0, 0, 0, 0, 0]);

    push(&link, sample(7, vec![(1, p(13108, 0, 0), true)]));
    push(&link, sample(8, vec![(1, p(13108, 0, 6554), true)]));
    client
        .step(&mut end, &[], false, 500)
        .expect("two samples in one step");
    assert_eq!(client.render_positions(525), vec![9831, 0, 0, 0, 0, 0]);
    assert_eq!(client.render_positions(600), vec![13108, 0, 6554, 0, 0, 0]);
}

// ---------------------------------------------------------------------------
// C2.
// ---------------------------------------------------------------------------

/// B-S4, Reading 4: a refused sample and a revoked entity are never drawn.
#[test]
fn only_permitted_positions_are_drawn() {
    let link = LocalLink::new(LINK_FRAMES);
    let mut end = link.client_end();
    let mut client = live();

    push(
        &link,
        v4(1, 5, vec![enter(1), enter(2), pos(1, p(0, 0, 0), true)]),
    );
    client.step(&mut end, &[], false, 0).expect("first frame");

    // n(2) has no anchor, so the replica refuses the sample and no buffer
    // exists for it.
    push(&link, sample(6, vec![(2, p(65536, 0, 0), false)]));
    client
        .step(&mut end, &[], false, 400)
        .expect("sample frame");
    assert_eq!(client.smoother().buffer(id(2)), None);
    assert_eq!(&client.render_positions(400)[3..6], &[0, 0, 0]);
    // Control: n(1)'s buffer exists before the revocation frame.
    assert!(client.smoother().buffer(id(1)).is_some());

    push(
        &link,
        v4(
            2,
            7,
            vec![DeltaOp::Position {
                entity: id(1),
                position: None,
            }],
        ),
    );
    client
        .step(&mut end, &[], false, 450)
        .expect("revocation frame");
    assert_eq!(client.smoother().buffer(id(1)), None);
    assert_eq!(client.render_positions(450), vec![0; 6]);
}

// ---------------------------------------------------------------------------
// C3.
// ---------------------------------------------------------------------------

/// B-S4: a fault keeps the frames before it; the faulting frame observes
/// nothing.
#[test]
fn a_fault_keeps_the_frames_before_it() {
    let link = LocalLink::new(LINK_FRAMES);
    let mut end = link.client_end();
    let mut client = live();

    push(
        &link,
        v4(1, 5, vec![enter(1), enter(2), pos(1, p(0, 0, 0), true)]),
    );
    client.step(&mut end, &[], false, 0).expect("first frame");

    push(&link, sample(6, vec![(1, p(6554, 0, 0), true)]));
    push(&link, vec![0xA1, 0xFF]);
    let fault = client
        .step(&mut end, &[], false, 400)
        .expect_err("the lane-1 frame is undecodable");
    assert_eq!(fault.code, BridgeCode::FrameUndecodable);
    assert_eq!(fault.code as i64, 52);

    let buffer = client.smoother().buffer(id(1)).expect("a buffer");
    let ticks: Vec<u64> = buffer.iter().map(|sample| sample.tick).collect();
    assert_eq!(ticks, vec![5, 6]);
    assert_eq!(client.smoother().offset_ms(), Some(100));
}

// ---------------------------------------------------------------------------
// E1.
// ---------------------------------------------------------------------------

/// B-S5: a real-time walk draws between samples, monotonically and bounded.
#[test]
fn walk_draws_between_samples() {
    let mut core = BridgeCore::new();
    let clock = ManualClock::new();
    assert_eq!(
        core.open_v4_rt_with(
            walk_root().to_str().expect("walk path"),
            "",
            "",
            1,
            clock.clone()
        ),
        BridgeCode::Ok
    );
    let (code, _views) = settle(&mut core, &clock, 0);
    assert_eq!(code, BridgeCode::Ok);
    assert_eq!(core.render_positions(), core.view().positions().to_vec());

    assert_eq!(core.move_to(1, 327680, 0, 327680), BridgeCode::Ok);
    let mut drawn = Vec::new();
    let mut held = Vec::new();
    let mut samples = BTreeSet::new();
    for k in 1..=40u64 {
        let (code, views) = settle(&mut core, &clock, 50 * k);
        assert_eq!(code, BridgeCode::Ok, "settle({})", 50 * k);
        for view in &views {
            samples.insert(view.positions()[0]);
        }
        drawn.push(core.render_positions()[0]);
        assert_eq!(core.poll(50 * k + 25), BridgeCode::Ok);
        let value = core.render_positions()[0];
        held.push(value);
        drawn.push(value);
    }

    for pair in drawn.windows(2) {
        assert!(pair[0] <= pair[1], "the drawn path went back: {pair:?}");
    }
    for value in &drawn {
        assert!(
            (196_608..=327_680).contains(value),
            "drawn outside the walk: {value}"
        );
    }
    assert!(
        held.iter()
            .any(|value| !samples.contains(value) && *value > 196_608 && *value < 327_680),
        "no drawn value was interpolated between two samples"
    );
    assert_eq!(core.render_positions(), core.view().positions().to_vec());
    core.close();
}

// ---------------------------------------------------------------------------
// E2.
// ---------------------------------------------------------------------------

/// B-S5: a fenced session draws its last view, not a dropped client's.
#[test]
fn a_fenced_session_draws_its_last_view() {
    let mut core = BridgeCore::new();
    let clock = ManualClock::new();
    assert_eq!(
        core.open_v4_rt_with(
            walk_root().to_str().expect("walk path"),
            "",
            "",
            1,
            clock.clone()
        ),
        BridgeCode::Ok
    );
    let (code, _views) = settle(&mut core, &clock, 0);
    assert_eq!(code, BridgeCode::Ok);
    assert_eq!(core.move_to(1, 327680, 0, 327680), BridgeCode::Ok);
    for k in 1..=5u64 {
        let (code, _views) = settle(&mut core, &clock, 50 * k);
        assert_eq!(code, BridgeCode::Ok, "settle({})", 50 * k);
    }
    let kept = core.view();

    core.probe = Some(Probe::Submit);
    let fenced = core.move_to(1, 0, 0, 0);
    assert_eq!(fenced, BridgeCode::Panicked);
    assert_eq!(fenced as i64, 55);
    assert_eq!(core.poll(300), BridgeCode::Panicked);
    assert_eq!(core.render_positions(), kept.positions().to_vec());
    core.close();
}
