//! T026p3 implementer suite: the protocol v4 position filter (`positions_v4`)
//! and the `Moved` event candidate (`projection_v4`), derived from
//! `tasks/T026p.md` Behaviour Q-1..Q-8 and the tests Q1..Q12, through the
//! public API only.
//!
//! Every expectation is a hand-written whole `DeltaOp` / `SampleEntry` value
//! or literal byte string, never produced by the code under test. Every
//! refusal is compared against pre-call clones of all three views and has a
//! passing control beside it (the same call with the fault removed).

use crpg_core::{GenerationalArena, Ulid};
use crpg_net::codec_v4::encode_delta;
use crpg_net::facts_v3::{
    project_page, project_snapshot_v3, EncounterGrants, FactContext, FactError, FactView,
    MAX_PAGE_OPS_V3,
};
use crpg_net::interest::{
    EntityFact, EntityKey, FieldMask, InterestError, InterestSet, InterestView, ViewCursor,
    ViewerContext,
};
use crpg_net::movement_v4::SampleEntry;
use crpg_net::positions_v4::{
    project_page_v4, project_samples, project_snapshot_v4, PageV4, PositionRow, PositionView,
    MAX_PAGE_OPS_V4, MAX_POSITION_PAGE_OPS,
};
use crpg_net::projection_v2::EventCandidate as V2Candidate;
use crpg_net::projection_v3::EventCandidate as V3Candidate;
use crpg_net::projection_v4::{project_events, EventCandidate, ProjectionError};
use crpg_net::protocol::{MAX_DELTA_OPS, MAX_VISIBLE_ENTITIES};
use crpg_net::protocol_v3::DeltaOp as V3Op;
use crpg_net::protocol_v4::{DeltaFrame, DeltaOp, NetId, Point, PositionFact, SessionEpoch};

// ---------------------------------------------------------------------------
// Fixtures.
// ---------------------------------------------------------------------------

fn u(n: u128) -> Ulid {
    Ulid::from_u128(n)
}

fn id(n: u64) -> NetId {
    NetId::new(n).expect("fixture net id is nonzero")
}

fn epoch() -> SessionEpoch {
    SessionEpoch([0; 16])
}

fn other_epoch() -> SessionEpoch {
    SessionEpoch([1; 16])
}

fn p(x: i32, y: i32, z: i32) -> Point {
    Point::new([x, y, z]).expect("fixture point is within U3")
}

/// `A = P(0, 0, 0)`.
fn pa() -> Point {
    p(0, 0, 0)
}

/// `B = P(65536, 0, -131072)`.
fn pb() -> Point {
    p(65536, 0, -131072)
}

/// `k3`'s point: `P(65536, 0, 65536)`.
fn pc() -> Point {
    p(65536, 0, 65536)
}

fn fact(point: Point, moving: bool) -> PositionFact {
    PositionFact { point, moving }
}

fn anchor(net: u64, point: Point, moving: bool) -> DeltaOp {
    DeltaOp::Position {
        entity: id(net),
        position: Some(fact(point, moving)),
    }
}

fn forget(net: u64) -> DeltaOp {
    DeltaOp::Position {
        entity: id(net),
        position: None,
    }
}

fn enter(net: u64) -> DeltaOp {
    DeltaOp::V3(V3Op::EntityEnter { entity: id(net) })
}

fn leave(net: u64) -> DeltaOp {
    DeltaOp::V3(V3Op::EntityLeave { entity: id(net) })
}

fn entry(net: u64, point: Point, moving: bool) -> SampleEntry {
    SampleEntry {
        entity: id(net),
        point,
        moving,
    }
}

/// `n` keys of one arena in area `U(area)`, ascending.
fn keys_in(area: u128, n: usize) -> Vec<EntityKey> {
    let mut arena = GenerationalArena::<()>::new();
    let keys: Vec<EntityKey> = (0..n)
        .map(|_| EntityKey {
            area: u(area),
            entity: arena.insert(()),
        })
        .collect();
    assert!(keys.windows(2).all(|w| w[0] < w[1]), "arena ids ascend");
    keys
}

fn presence(key: EntityKey, on: bool) -> EntityFact {
    EntityFact {
        key,
        fields: FieldMask {
            presence: on,
            health: false,
        },
        health: None,
        resend_health: false,
    }
}

fn row(key: EntityKey, granted: bool, point: Point, moving: bool) -> PositionRow {
    PositionRow {
        key,
        granted,
        point,
        moving,
    }
}

/// The delta frame of one page, for byte comparison.
fn bytes(ops: &[DeltaOp]) -> Vec<u8> {
    encode_delta(&DeltaFrame {
        lane: 0,
        server_tick: 100,
        event_seq: 1,
        ops: ops.to_vec(),
    })
    .expect("a fixture page encodes")
}

/// Everything one call reads, plus the three per-viewer views.
#[derive(Clone)]
struct Scene {
    keys: Vec<EntityKey>,
    state_epoch: SessionEpoch,
    state: Vec<EntityFact>,
    despawned: Vec<EntityKey>,
    rows: Vec<PositionRow>,
    resend: bool,
    view: InterestView,
    facts: FactView,
    positions: PositionView,
}

impl Scene {
    /// The contract fixture: `k1`, `k2`, `k3` present, nothing else granted;
    /// position rows `k1 { granted, A, false }`, `k2 { granted, B, true }`,
    /// `k3 { not granted, P(65536, 0, 65536), false }`. Nothing is projected
    /// yet.
    fn new() -> Self {
        let keys = keys_in(10, 3);
        Self {
            state: keys.iter().map(|k| presence(*k, true)).collect(),
            rows: vec![
                row(keys[0], true, pa(), false),
                row(keys[1], true, pb(), true),
                row(keys[2], false, pc(), false),
            ],
            keys,
            state_epoch: epoch(),
            despawned: Vec::new(),
            resend: false,
            view: InterestView::new(epoch()),
            facts: FactView::new(epoch()),
            positions: PositionView::new(epoch()),
        }
    }

    /// The fixture after its first page (Q2's page).
    fn after_q2() -> Self {
        let mut s = Self::new();
        s.page().expect("the first page is Ok");
        s
    }

    fn k(&self, n: usize) -> EntityKey {
        self.keys[n - 1]
    }

    /// Sets the row of `k(n)`.
    fn set_row(&mut self, n: usize, granted: bool, point: Point, moving: bool) {
        let key = self.k(n);
        let r = self
            .rows
            .iter_mut()
            .find(|r| r.key == key)
            .expect("fixture row exists");
        *r = row(key, granted, point, moving);
    }

    fn drop_row(&mut self, n: usize) {
        let key = self.k(n);
        self.rows.retain(|r| r.key != key);
    }

    fn set_presence(&mut self, n: usize, on: bool) {
        let key = self.k(n);
        let f = self
            .state
            .iter_mut()
            .find(|f| f.key == key)
            .expect("fixture state row exists");
        f.fields.presence = on;
    }

    fn state_ctx(&self) -> ViewerContext<'_> {
        ViewerContext {
            epoch: self.state_epoch,
            tick: 9,
            interest: InterestSet { area: u(10) },
            facts: &self.state,
            despawned: &self.despawned,
            turn: None,
            turn_granted: false,
            resend_turn: false,
        }
    }

    fn fact_ctx(&self) -> FactContext<'static> {
        FactContext {
            rows: &[],
            encounter: None,
            grants: EncounterGrants::default(),
            resend: self.resend,
        }
    }

    fn page(&mut self) -> Result<PageV4, FactError> {
        let Scene {
            state_epoch,
            state,
            despawned,
            rows,
            resend,
            view,
            facts,
            positions,
            ..
        } = self;
        let state = ViewerContext {
            epoch: *state_epoch,
            tick: 9,
            interest: InterestSet { area: u(10) },
            facts: state,
            despawned,
            turn: None,
            turn_granted: false,
            resend_turn: false,
        };
        let context = FactContext {
            rows: &[],
            encounter: None,
            grants: EncounterGrants::default(),
            resend: *resend,
        };
        project_page_v4(view, facts, positions, &state, &context, rows)
    }

    fn samples(&mut self, refresh: bool) -> Result<Vec<SampleEntry>, FactError> {
        project_samples(&self.view, &mut self.positions, &self.rows, refresh)
    }

    fn snapshot(&mut self) -> Result<crpg_net::facts_v3::SnapshotProjectionV3, FactError> {
        let state = ViewerContext {
            epoch: self.state_epoch,
            tick: 9,
            interest: InterestSet { area: u(10) },
            facts: &self.state,
            despawned: &self.despawned,
            turn: None,
            turn_granted: false,
            resend_turn: false,
        };
        let context = self.fact_ctx();
        project_snapshot_v4(
            &mut self.view,
            &mut self.facts,
            &mut self.positions,
            &state,
            &context,
        )
    }

    /// `page` that must fail, with all three views checked against clones.
    fn page_err(&mut self) -> FactError {
        let before = (
            self.view.clone(),
            self.facts.clone(),
            self.positions.clone(),
        );
        let err = self.page().expect_err("the page must be refused");
        assert_eq!(self.view, before.0, "view changed on Err");
        assert_eq!(self.facts, before.1, "facts changed on Err");
        assert_eq!(self.positions, before.2, "positions changed on Err");
        err
    }

    /// `samples` that must fail, with `positions` checked against its clone.
    fn samples_err(&mut self, refresh: bool) -> FactError {
        let view = self.view.clone();
        let positions = self.positions.clone();
        let err = self.samples(refresh).expect_err("samples must be refused");
        assert_eq!(self.view, view);
        assert_eq!(self.positions, positions, "positions changed on Err");
        err
    }

    fn snapshot_err(&mut self) -> FactError {
        let before = (
            self.view.clone(),
            self.facts.clone(),
            self.positions.clone(),
        );
        let err = self.snapshot().expect_err("the snapshot must be refused");
        assert_eq!(self.view, before.0, "view changed on Err");
        assert_eq!(self.facts, before.1, "facts changed on Err");
        assert_eq!(self.positions, before.2, "positions changed on Err");
        err
    }
}

/// The contract's first page for the fixture (Q2).
fn q2_ops() -> Vec<DeltaOp> {
    vec![
        enter(1),
        enter(2),
        enter(3),
        anchor(1, pa(), false),
        anchor(2, pb(), true),
    ]
}

/// `extra` more rows in a second area, so a row list can pass the bound.
fn filler_rows(n: usize) -> Vec<PositionRow> {
    keys_in(11, n)
        .into_iter()
        .map(|k| row(k, false, pa(), false))
        .collect()
}

// ---------------------------------------------------------------------------
// Q1.
// ---------------------------------------------------------------------------

#[test]
fn q1_checks_are_all_or_nothing_for_pages() {
    // Control: the unfaulted first page is Ok.
    assert_eq!(Scene::new().page().expect("control").ops, q2_ops());

    // A PositionView of another epoch.
    let mut s = Scene::new();
    s.positions = PositionView::new(other_epoch());
    assert_eq!(s.page_err(), FactError::WrongEpoch);
    s.positions = PositionView::new(epoch());
    assert_eq!(s.page().expect("fixed").ops, q2_ops());

    // 1,025 rows, with and without a duplicate; 1,024 rows is the control.
    let mut s = Scene::new();
    s.rows.extend(filler_rows(MAX_VISIBLE_ENTITIES - 3));
    assert_eq!(s.rows.len(), 1024);
    let ok = s.clone().page().expect("1,024 rows are accepted");
    assert_eq!(ok.ops, q2_ops());
    s.rows.extend(filler_rows(1));
    assert_eq!(s.rows.len(), 1025);
    assert_eq!(s.page_err(), FactError::TooManyFacts);
    let dup = s.rows[0];
    s.rows[1024] = dup;
    assert_eq!(s.page_err(), FactError::TooManyFacts);
    s.rows.pop();
    assert_eq!(s.page().expect("fixed").ops, q2_ops());

    // A duplicate row key.
    let mut s = Scene::new();
    s.rows.push(s.rows[0]);
    assert_eq!(s.page_err(), FactError::DuplicateFact);
    s.rows.pop();
    assert_eq!(s.page().expect("fixed").ops, q2_ops());

    // A state of another epoch.
    let mut s = Scene::new();
    s.state_epoch = other_epoch();
    assert_eq!(s.page_err(), FactError::WrongEpoch);
    s.state_epoch = epoch();
    assert_eq!(s.page().expect("fixed").ops, q2_ops());

    // A state with two rows for k1.
    let mut s = Scene::new();
    s.state.push(s.state[0].clone());
    assert_eq!(
        s.page_err(),
        FactError::Interest(InterestError::DuplicateFact)
    );
    s.state.pop();
    assert_eq!(s.page().expect("fixed").ops, q2_ops());
}

#[test]
fn q1_two_faults_first_wins_for_pages() {
    // A foreign PositionView with 1,025 rows is WrongEpoch.
    let mut s = Scene::new();
    s.positions = PositionView::new(other_epoch());
    s.rows.extend(filler_rows(1022));
    assert_eq!(s.rows.len(), 1025);
    assert_eq!(s.page_err(), FactError::WrongEpoch);

    // 1,025 rows with a state of another epoch is TooManyFacts.
    let mut s = Scene::new();
    s.rows.extend(filler_rows(1022));
    s.state_epoch = other_epoch();
    assert_eq!(s.page_err(), FactError::TooManyFacts);

    // Duplicate rows with a state of another epoch is DuplicateFact.
    let mut s = Scene::new();
    s.rows.push(s.rows[0]);
    s.state_epoch = other_epoch();
    assert_eq!(s.page_err(), FactError::DuplicateFact);

    // The control for the last: the state's own fault shows once the
    // position checks pass.
    let mut s = Scene::new();
    s.state_epoch = other_epoch();
    assert_eq!(s.page_err(), FactError::WrongEpoch);

    // A foreign epoch with a duplicate is WrongEpoch (epoch first).
    let mut s = Scene::new();
    s.positions = PositionView::new(other_epoch());
    s.rows.push(s.rows[0]);
    assert_eq!(s.page_err(), FactError::WrongEpoch);
}

#[test]
fn q1_samples_checks_and_precedence() {
    // Memory is non-empty and a value has changed, so a refusal that mutated
    // anything would show in the clone comparison and in the control.
    let mut base = Scene::after_q2();
    base.set_row(2, true, p(131072, 0, 0), true);
    let expected = vec![entry(2, p(131072, 0, 0), true)];

    let mut s = base.clone();
    assert_eq!(s.samples(false).expect("control"), expected);

    let mut s = base.clone();
    s.positions = PositionView::new(other_epoch());
    assert_eq!(s.samples_err(false), FactError::WrongEpoch);

    let mut s = base.clone();
    s.rows.extend(filler_rows(1022));
    assert_eq!(s.samples_err(false), FactError::TooManyFacts);
    s.rows.truncate(1024);
    assert_eq!(s.samples(false).expect("1,024 rows pass"), expected);

    let mut s = base.clone();
    s.rows.push(s.rows[0]);
    assert_eq!(s.samples_err(false), FactError::DuplicateFact);
    s.rows.pop();
    assert_eq!(s.samples(false).expect("fixed"), expected);

    // Two faults: a foreign PositionView with 1,025 rows is WrongEpoch.
    let mut s = base.clone();
    s.positions = PositionView::new(other_epoch());
    s.rows.extend(filler_rows(1022));
    assert_eq!(s.samples_err(false), FactError::WrongEpoch);

    // 1,025 rows with a duplicate is TooManyFacts.
    let mut s = base.clone();
    s.rows.extend(filler_rows(1022));
    let dup = s.rows[0];
    s.rows[1024] = dup;
    assert_eq!(s.samples_err(false), FactError::TooManyFacts);
}

// ---------------------------------------------------------------------------
// Q2..Q6, Q8.
// ---------------------------------------------------------------------------

#[test]
fn q2_first_page_anchors() {
    let mut s = Scene::new();
    let page = s.page().expect("first page");
    assert_eq!(page.ops, q2_ops());
    assert_eq!(page.enters, 3);
    assert_eq!(
        page.cursor,
        ViewCursor {
            tick: 9,
            revision: 1
        }
    );
    assert_eq!(page.cursor, s.view.cursor());
}

#[test]
fn q3_values_travel_in_samples() {
    let mut s = Scene::after_q2();
    assert_eq!(s.page().expect("page").ops, vec![]);
    assert_eq!(s.samples(false).expect("samples"), vec![]);

    let moved = p(131072, 0, 0);
    s.set_row(2, true, moved, true);
    assert_eq!(s.page().expect("page").ops, vec![], "page carries no value");
    assert_eq!(s.samples(false).expect("s"), vec![entry(2, moved, true)]);
    assert_eq!(s.samples(false).expect("s"), vec![], "unchanged: no repeat");

    // moving changes at the same point.
    s.set_row(2, true, moved, false);
    assert_eq!(s.samples(false).expect("s"), vec![entry(2, moved, false)]);
    assert_eq!(s.samples(false).expect("s"), vec![]);

    // refresh re-sends every granted remembered value, ascending.
    assert_eq!(
        s.samples(true).expect("s"),
        vec![entry(1, pa(), false), entry(2, moved, false)]
    );
    // A refresh updates the memory too: the next plain call is empty.
    assert_eq!(s.samples(false).expect("s"), vec![]);
}

#[test]
fn q3_sample_changed_by_point_alone_and_by_moving_alone() {
    let mut s = Scene::after_q2();
    // Same point, moving flipped.
    s.set_row(2, true, pb(), false);
    assert_eq!(s.samples(false).expect("s"), vec![entry(2, pb(), false)]);
    // Same moving, point changed on k1.
    s.set_row(1, true, p(1, 2, 3), false);
    assert_eq!(
        s.samples(false).expect("s"),
        vec![entry(1, p(1, 2, 3), false)]
    );
    assert_eq!(s.samples(false).expect("s"), vec![]);
}

#[test]
fn q4_revoke_after_disclosure() {
    let mut s = Scene::after_q2();
    s.set_row(2, false, pb(), true);
    // Revocation is immediate: before any page.
    assert_eq!(s.samples(true).expect("s"), vec![entry(1, pa(), false)]);
    // The reliable forget follows, once.
    assert_eq!(s.page().expect("page").ops, vec![forget(2)]);
    assert_eq!(s.page().expect("page").ops, vec![]);
    assert_eq!(s.samples(true).expect("s"), vec![entry(1, pa(), false)]);

    // Re-grant at a new point: re-anchored; the sample after is empty.
    s.set_row(2, true, p(0, 0, 65536), true);
    assert_eq!(
        s.page().expect("page").ops,
        vec![anchor(2, p(0, 0, 65536), true)]
    );
    assert_eq!(s.samples(false).expect("s"), vec![]);
}

#[test]
fn q4_revoked_values_move_no_byte() {
    // Baseline: revoke k2, page, with k2's row never changing.
    let mut base = Scene::after_q2();
    base.set_row(2, false, pb(), true);
    let base_samples = base.samples(true).expect("s");
    let base_page = base.page().expect("page").ops;
    assert_eq!(base_page, vec![forget(2)]);
    assert_eq!(
        bytes(&base_page),
        [0x04, 0x00, 0x64, 0x01, 0x01, 0x16, 0x02, 0x00]
    );
    let base_later = base.page().expect("page").ops;
    assert_eq!(bytes(&base_later), [0x04, 0x00, 0x64, 0x01, 0x00]);

    // Before the None page, the revoked row moves.
    let mut s = Scene::after_q2();
    s.set_row(2, false, p(0, 0, 131072), true);
    assert_eq!(s.samples(true).expect("s"), base_samples);
    let page = s.page().expect("page").ops;
    assert_eq!(bytes(&page), bytes(&base_page));
    // After the None page, it moves again (and flips moving).
    s.set_row(2, false, p(7, 8, 9), false);
    let page = s.page().expect("page").ops;
    assert_eq!(bytes(&page), bytes(&base_later));
    assert_eq!(s.samples(true).expect("s"), base_samples);
    assert_eq!(s.samples(true).expect("s"), vec![entry(1, pa(), false)]);
}

#[test]
fn q5_leave_and_reentry() {
    let mut s = Scene::after_q2();
    s.set_presence(2, false);
    // The leave carries no Position op.
    assert_eq!(s.page().expect("page").ops, vec![leave(2)]);
    assert_eq!(s.samples(true).expect("s"), vec![entry(1, pa(), false)]);
    // The row is still granted; presence back on gives a new id and a new
    // anchor (the old memory does not suppress it).
    s.set_presence(2, true);
    assert_eq!(
        s.page().expect("page").ops,
        vec![enter(4), anchor(4, pb(), true)]
    );
    assert_eq!(
        s.samples(true).expect("s"),
        vec![entry(1, pa(), false), entry(4, pb(), true)]
    );
}

#[test]
fn q5_removed_key_and_despawn() {
    // A key dropped from the state is a leave.
    let mut s = Scene::after_q2();
    let k2 = s.k(2);
    s.state.retain(|f| f.key != k2);
    assert_eq!(s.page().expect("page").ops, vec![leave(2)]);
    assert_eq!(s.samples(true).expect("s"), vec![entry(1, pa(), false)]);

    // A key dropped and listed as despawned is a despawn, with no Position.
    let mut s = Scene::after_q2();
    let k2 = s.k(2);
    s.state.retain(|f| f.key != k2);
    s.despawned.push(k2);
    assert_eq!(
        s.page().expect("page").ops,
        vec![DeltaOp::V3(V3Op::Despawned { entity: id(2) })]
    );
    assert_eq!(s.samples(true).expect("s"), vec![entry(1, pa(), false)]);
    // Control: it is the memory of 2 that was dropped, not 1's.
    s.set_row(1, true, pb(), true);
    assert_eq!(s.samples(false).expect("s"), vec![entry(1, pb(), true)]);
}

#[test]
fn q5_a_view_that_reissued_the_id_forgets_the_old_memory() {
    // The memory says k2 was told under id 2. A view that now maps k2 to id 1
    // (k1 absent) makes that memory stale: no sample from it, and the page
    // anchors k2 afresh under its new id.
    let mut s = Scene::after_q2();
    let mut fresh_view = InterestView::new(epoch());
    let mut fresh_facts = FactView::new(epoch());
    s.set_presence(1, false);
    {
        let state = s.state_ctx();
        let context = s.fact_ctx();
        let page = project_page(&mut fresh_view, &mut fresh_facts, &state, &context)
            .expect("v3 page on the new view");
        assert_eq!(page.ops.len(), 2, "k2 -> 1 and k3 -> 2 enter");
    }
    assert_eq!(fresh_view.net_id(s.k(2)), Some(id(1)));
    assert_eq!(fresh_view.net_id(s.k(1)), None);

    // Samples: id mismatch for k2 and no id for k1 give no entry and no
    // change of memory.
    let before = s.positions.clone();
    let out = project_samples(&fresh_view, &mut s.positions, &s.rows, true).expect("samples");
    assert_eq!(out, vec![]);
    assert_eq!(s.positions, before, "stale memory is kept, not rewritten");
    // Control: the view the memory was made under samples both.
    assert_eq!(
        s.samples(true).expect("s"),
        vec![entry(1, pa(), false), entry(2, pb(), true)]
    );

    // Page: the stale memory is dropped and k2 is anchored under id 1.
    s.view = fresh_view;
    s.facts = fresh_facts;
    s.set_row(1, true, pa(), false);
    // k1 is still absent from the state, so it never becomes visible.
    let page = s.page().expect("page");
    assert_eq!(page.ops, vec![anchor(1, pb(), true)]);
    assert_eq!(s.samples(true).expect("s"), vec![entry(1, pb(), true)]);
}

#[test]
fn q6_late_grant_and_rowless_key() {
    let mut s = Scene::after_q2();
    s.set_row(3, true, pc(), false);
    assert_eq!(s.page().expect("page").ops, vec![anchor(3, pc(), false)]);

    // k1's row removed: its remembered value is not re-sent.
    s.drop_row(1);
    assert_eq!(
        s.samples(true).expect("s"),
        vec![entry(2, pb(), true), entry(3, pc(), false)]
    );
    assert_eq!(s.page().expect("page").ops, vec![forget(1)]);
    assert_eq!(s.page().expect("page").ops, vec![]);
    // The row coming back is a fresh anchor.
    s.set_row_back_for_k1();
    assert_eq!(s.page().expect("page").ops, vec![anchor(1, pa(), false)]);
}

impl Scene {
    fn set_row_back_for_k1(&mut self) {
        self.rows.push(row(self.keys[0], true, pa(), false));
    }
}

#[test]
fn q6_rows_of_invisible_keys_are_ignored() {
    // A granted row for a key that is not in the state, and one that is
    // present but without presence, give no op and no sample.
    let mut s = Scene::new();
    let stranger = keys_in(10, 5)[4];
    s.rows.push(row(stranger, true, pb(), true));
    s.set_presence(3, false);
    s.set_row(3, true, pc(), false);
    assert_eq!(
        s.page().expect("page").ops,
        vec![
            enter(1),
            enter(2),
            anchor(1, pa(), false),
            anchor(2, pb(), true)
        ]
    );
    assert_eq!(
        s.samples(true).expect("s"),
        vec![entry(1, pa(), false), entry(2, pb(), true)]
    );
}

#[test]
fn q7_samples_never_add_keys() {
    let mut s = Scene::new();
    assert_eq!(s.samples(true).expect("fresh"), vec![]);
    assert_eq!(s.positions, PositionView::new(epoch()), "nothing added");

    // After a page with k2 not granted, k2 granted is still not sampled.
    let mut s = Scene::new();
    s.set_row(2, false, pb(), true);
    assert_eq!(
        s.page().expect("page").ops,
        vec![enter(1), enter(2), enter(3), anchor(1, pa(), false)]
    );
    s.set_row(2, true, pb(), true);
    assert_eq!(s.samples(true).expect("s"), vec![entry(1, pa(), false)]);
    assert_eq!(s.samples(true).expect("s"), vec![entry(1, pa(), false)]);
    // Control: its page anchors it, then it is sampled.
    assert_eq!(s.page().expect("page").ops, vec![anchor(2, pb(), true)]);
    assert_eq!(
        s.samples(true).expect("s"),
        vec![entry(1, pa(), false), entry(2, pb(), true)]
    );
}

#[test]
fn q8_resend() {
    let mut s = Scene::after_q2();
    let moved = p(131072, 0, 0);
    s.set_row(2, true, moved, true);
    s.resend = true;
    assert_eq!(
        s.page().expect("page").ops,
        vec![anchor(1, pa(), false), anchor(2, moved, true)]
    );
    // The resend remembered the new value.
    assert_eq!(s.samples(false).expect("s"), vec![]);

    // A plain page afterwards is empty again.
    s.resend = false;
    assert_eq!(s.page().expect("page").ops, vec![]);
}

#[test]
fn q8_resend_with_a_revoked_and_a_late_grant() {
    let mut s = Scene::after_q2();
    s.resend = true;
    s.set_row(2, false, pb(), true);
    s.set_row(3, true, pc(), false);
    // k1 re-anchored, k2 forgotten, k3 anchored (nothing remembered).
    assert_eq!(
        s.page().expect("page").ops,
        vec![anchor(1, pa(), false), forget(2), anchor(3, pc(), false)]
    );
    // A second resend with nothing remembered for 2 gives no forget.
    assert_eq!(
        s.page().expect("page").ops,
        vec![anchor(1, pa(), false), anchor(3, pc(), false)]
    );
}

#[test]
fn q4_emission_follows_net_id_not_key_order() {
    // k1 enters last (id 3), so net order is k2, k3, k1; key order is the
    // reverse of what the ops must show.
    let mut s = Scene::new();
    s.set_presence(1, false);
    s.rows = vec![
        row(s.k(1), false, pa(), false),
        row(s.k(2), false, pb(), true),
        row(s.k(3), false, pc(), false),
    ];
    assert_eq!(s.page().expect("page").ops, vec![enter(1), enter(2)]);
    s.set_presence(1, true);
    assert_eq!(s.page().expect("page").ops, vec![enter(3)]);
    assert_eq!(s.view.net_id(s.k(2)), Some(id(1)));
    assert_eq!(s.view.net_id(s.k(3)), Some(id(2)));
    assert_eq!(s.view.net_id(s.k(1)), Some(id(3)));

    for n in 1..=3 {
        let r = s.rows.iter().find(|r| r.key == s.k(n)).copied().unwrap();
        s.set_row(n, true, r.point, r.moving);
    }
    assert_eq!(
        s.page().expect("page").ops,
        vec![
            anchor(1, pb(), true),
            anchor(2, pc(), false),
            anchor(3, pa(), false)
        ]
    );
    // Samples ascend by net id too.
    for n in 1..=3 {
        s.set_row(n, true, p(n as i32, 0, 0), true);
    }
    assert_eq!(
        s.samples(false).expect("s"),
        vec![
            entry(1, p(2, 0, 0), true),
            entry(2, p(3, 0, 0), true),
            entry(3, p(1, 0, 0), true)
        ]
    );
    s.resend = true;
    assert_eq!(
        s.page().expect("page").ops,
        vec![
            anchor(1, p(2, 0, 0), true),
            anchor(2, p(3, 0, 0), true),
            anchor(3, p(1, 0, 0), true)
        ]
    );
}

// ---------------------------------------------------------------------------
// Q9.
// ---------------------------------------------------------------------------

#[test]
fn q9_snapshot_clears_memory() {
    let mut s = Scene::after_q2();
    let mut view = s.view.clone();
    let mut facts = s.facts.clone();
    let expected = {
        let state = s.state_ctx();
        let context = s.fact_ctx();
        project_snapshot_v3(&mut view, &mut facts, &state, &context).expect("v3 snapshot")
    };
    let got = s.snapshot().expect("v4 snapshot");
    assert_eq!(got, expected);
    assert_eq!(s.view, view);
    assert_eq!(s.facts, facts);
    assert_eq!(s.positions, PositionView::new(epoch()), "memory is empty");

    assert_eq!(s.samples(true).expect("s"), vec![]);
    assert_eq!(
        s.page().expect("page").ops,
        vec![anchor(1, pa(), false), anchor(2, pb(), true)]
    );
    // After the re-anchor, samples flow again.
    assert_eq!(
        s.samples(true).expect("s"),
        vec![entry(1, pa(), false), entry(2, pb(), true)]
    );
}

#[test]
fn q9_snapshot_refusals_change_nothing() {
    // A PositionView of another epoch.
    let mut s = Scene::after_q2();
    s.positions = PositionView::new(other_epoch());
    assert_eq!(s.snapshot_err(), FactError::WrongEpoch);

    // A state of another epoch: the v3 error, memory not cleared.
    let mut s = Scene::after_q2();
    s.state_epoch = other_epoch();
    assert_eq!(s.snapshot_err(), FactError::WrongEpoch);
    s.state_epoch = epoch();
    assert_eq!(
        s.samples(true).expect("memory survived"),
        vec![entry(1, pa(), false), entry(2, pb(), true)]
    );

    // A duplicate state row: the v3 error, memory not cleared.
    let mut s = Scene::after_q2();
    s.state.push(s.state[0].clone());
    assert_eq!(
        s.snapshot_err(),
        FactError::Interest(InterestError::DuplicateFact)
    );
    s.state.pop();
    assert_eq!(
        s.samples(true).expect("memory survived"),
        vec![entry(1, pa(), false), entry(2, pb(), true)]
    );
    // Control: the same scene snapshots Ok.
    assert!(s.snapshot().is_ok());
}

// ---------------------------------------------------------------------------
// Q10 and Q12.
// ---------------------------------------------------------------------------

/// Drives the same inputs through `project_page_v4` and, in parallel, the v3
/// `project_page`, comparing the pages.
fn run_against_v3(s: &mut Scene, v3: &mut (InterestView, FactView)) -> PageV4 {
    let page = s.page().expect("v4 page");
    let state = s.state_ctx();
    let context = s.fact_ctx();
    let reference = project_page(&mut v3.0, &mut v3.1, &state, &context).expect("v3 page");
    let wrapped: Vec<DeltaOp> = reference.ops.into_iter().map(DeltaOp::from).collect();
    assert_eq!(page.ops, wrapped, "no position op for a no-grant viewer");
    assert_eq!(page.enters, reference.enters);
    assert_eq!(page.cursor, reference.cursor);
    page
}

#[test]
fn q10_no_grant_viewer_sees_no_position() {
    for rows_present in [true, false] {
        let mut s = Scene::new();
        for n in 1..=3 {
            let r = *s.rows.iter().find(|r| r.key == s.k(n)).unwrap();
            s.set_row(n, false, r.point, r.moving);
        }
        if !rows_present {
            s.rows.clear();
        }
        let mut v3 = (InterestView::new(epoch()), FactView::new(epoch()));

        let page = run_against_v3(&mut s, &mut v3);
        assert_eq!(page.ops, vec![enter(1), enter(2), enter(3)]);
        assert_eq!(s.samples(true).expect("s"), vec![]);

        // Value changes: still nothing, page or sample.
        for step in 1..=3i32 {
            if rows_present {
                s.set_row(1, false, p(step, 0, 0), step % 2 == 0);
                s.set_row(2, false, p(0, step, 0), step % 2 == 1);
                s.set_row(3, false, p(0, 0, step), true);
            }
            let page = run_against_v3(&mut s, &mut v3);
            assert_eq!(page.ops, vec![]);
            assert_eq!(s.samples(true).expect("s"), vec![]);
            assert_eq!(s.samples(false).expect("s"), vec![]);
        }
        // A resend page too.
        s.resend = true;
        let page = run_against_v3(&mut s, &mut v3);
        assert_eq!(page.ops, vec![]);
        assert_eq!(s.samples(true).expect("s"), vec![]);
    }
}

#[test]
fn q10_control_the_same_scene_with_grants_discloses() {
    let mut s = Scene::new();
    let page = s.page().expect("page");
    assert_eq!(page.ops, q2_ops());
    assert_eq!(
        s.samples(true).expect("s"),
        vec![entry(1, pa(), false), entry(2, pb(), true)]
    );
}

#[test]
fn q12_hidden_values_move_no_bytes() {
    // k3 never granted; its point and moving change between calls.
    let mut base = Scene::new();
    let mut moved = Scene::new();
    let mut base_pages = Vec::new();
    let mut moved_pages = Vec::new();
    let mut base_samples = Vec::new();
    let mut moved_samples = Vec::new();
    for step in 0..4i32 {
        if step > 0 {
            moved.set_row(3, false, p(step, step, step), step % 2 == 0);
        }
        // Visible values change on both scenes alike.
        for scene in [&mut base, &mut moved] {
            scene.set_row(2, true, p(step * 65536, 0, 0), step % 2 == 0);
            scene.resend = step == 3;
        }
        base_pages.push(bytes(&base.page().expect("page").ops));
        moved_pages.push(bytes(&moved.page().expect("page").ops));
        base_samples.push(base.samples(step == 2).expect("s"));
        moved_samples.push(moved.samples(step == 2).expect("s"));
    }
    assert_eq!(base_pages, moved_pages);
    assert_eq!(base_samples, moved_samples);
    // Control: the visible values did move bytes, so the comparison bites.
    assert_ne!(base_pages[0], base_pages[1]);
    assert_eq!(
        base_samples[1],
        vec![entry(2, p(65536, 0, 0), false)],
        "k2 sampled at its changed value"
    );
    // And granting k3 does change the bytes.
    let mut granted = Scene::new();
    granted.set_row(3, true, pc(), false);
    assert_ne!(
        bytes(&granted.page().expect("page").ops),
        bytes(&Scene::new().page().expect("page").ops)
    );
}

// ---------------------------------------------------------------------------
// Bounds.
// ---------------------------------------------------------------------------

#[test]
fn constants_and_the_largest_page() {
    assert_eq!(MAX_POSITION_PAGE_OPS, 1024);
    assert_eq!(MAX_PAGE_OPS_V4, MAX_PAGE_OPS_V3 + 1024);
    assert_eq!(MAX_PAGE_OPS_V4, 5123);

    // 1,024 visible granted keys: 1,024 enters then 1,024 anchors, ascending.
    let keys = keys_in(10, MAX_VISIBLE_ENTITIES);
    let mut s = Scene::new();
    s.state = keys.iter().map(|k| presence(*k, true)).collect();
    s.rows = keys
        .iter()
        .enumerate()
        .map(|(i, k)| row(*k, true, p(i as i32, 0, 0), i % 2 == 0))
        .collect();
    s.keys = keys;
    let page = s.page().expect("page");
    assert_eq!(page.ops.len(), 2048);
    assert!(page.ops.len() <= MAX_PAGE_OPS_V4);
    assert_eq!(page.enters, 1024);
    for i in 0..1024usize {
        assert_eq!(page.ops[i], enter(i as u64 + 1));
        assert_eq!(
            page.ops[1024 + i],
            anchor(i as u64 + 1, p(i as i32, 0, 0), i % 2 == 0)
        );
    }
    let samples = s.samples(true).expect("s");
    assert_eq!(samples.len(), 1024);
    for (i, e) in samples.iter().enumerate() {
        assert_eq!(*e, entry(i as u64 + 1, p(i as i32, 0, 0), i % 2 == 0));
    }
}

// ---------------------------------------------------------------------------
// Q11.
// ---------------------------------------------------------------------------

fn moved(entity: Option<u64>, points: Vec<Point>, disclose: bool) -> EventCandidate {
    EventCandidate::Moved {
        entity: entity.map(id),
        points,
        disclose,
    }
}

fn round_started(round: Option<u64>) -> EventCandidate {
    EventCandidate::V3(V3Candidate::RoundStarted { round })
}

#[test]
fn q11_moved_projection() {
    let candidates = vec![
        EventCandidate::V3(V3Candidate::V2(V2Candidate::TurnStarted {
            actor: Some(id(1)),
            round: Some(1),
        })),
        moved(Some(2), vec![pa(), pb()], true),
        moved(Some(2), vec![pa(), pb()], false),
        moved(None, vec![pa(), pb()], true),
        moved(Some(2), vec![pa()], true),
        moved(Some(2), vec![pa(), p(0, 65536, 0)], true),
        round_started(Some(2)),
    ];
    assert_eq!(
        project_events(&candidates).expect("ok"),
        vec![
            DeltaOp::V3(V3Op::TurnStarted {
                actor: id(1),
                round: 1
            }),
            DeltaOp::Moved {
                entity: id(2),
                points: vec![pa(), pb()]
            },
            DeltaOp::V3(V3Op::RoundStarted { round: 2 }),
        ]
    );
}

#[test]
fn q11_moved_point_bounds_and_branches() {
    // 64 points are accepted, 65 are omitted, 0 and 1 are omitted.
    let l = 1 << 29;
    let route = |n: i32| -> Vec<Point> { (0..n).map(|i| p(l - i, l, l)).collect() };
    assert_eq!(
        project_events(&[moved(Some(5), route(64), true)]).expect("ok"),
        vec![DeltaOp::Moved {
            entity: id(5),
            points: route(64)
        }]
    );
    for n in [0, 1, 65] {
        assert_eq!(
            project_events(&[moved(Some(5), route(n), true)]).expect("ok"),
            vec![],
            "{n} points are withheld"
        );
    }
    // A zero horizontal segment is withheld wherever it is; a vertical step
    // alone is a zero segment, a horizontal one is not.
    assert_eq!(
        project_events(&[moved(Some(5), vec![pa(), p(1, 0, 0), p(1, 5, 0)], true)]).expect("ok"),
        vec![]
    );
    assert_eq!(
        project_events(&[moved(Some(5), vec![pa(), p(1, 0, 0), p(1, 5, 1)], true)]).expect("ok"),
        vec![DeltaOp::Moved {
            entity: id(5),
            points: vec![pa(), p(1, 0, 0), p(1, 5, 1)]
        }]
    );
    // No sorting and no deduplication: input order, repeats kept.
    let out = project_events(&[
        moved(Some(9), vec![pa(), pb()], true),
        moved(Some(3), vec![pa(), pb()], true),
        moved(Some(9), vec![pa(), pb()], true),
    ])
    .expect("ok");
    let nets: Vec<NetId> = out
        .iter()
        .map(|op| match op {
            DeltaOp::Moved { entity, .. } => *entity,
            other => panic!("unexpected op {other:?}"),
        })
        .collect();
    assert_eq!(nets, vec![id(9), id(3), id(9)]);
    // A v3 candidate keeps its position among Moved ones.
    assert_eq!(
        project_events(&[
            round_started(Some(1)),
            moved(Some(2), vec![pa(), pb()], true),
            round_started(Some(1)),
            round_started(None),
        ])
        .expect("ok"),
        vec![
            DeltaOp::V3(V3Op::RoundStarted { round: 1 }),
            DeltaOp::Moved {
                entity: id(2),
                points: vec![pa(), pb()]
            },
            DeltaOp::V3(V3Op::RoundStarted { round: 1 }),
        ]
    );
    assert_eq!(project_events(&[]).expect("ok"), vec![]);
}

#[test]
fn q11_count_cap_and_v3_errors() {
    let cap = MAX_DELTA_OPS;
    assert_eq!(cap, 256);
    let ok: Vec<EventCandidate> = (0..cap)
        .map(|_| moved(Some(2), vec![pa(), pb()], true))
        .collect();
    assert_eq!(project_events(&ok).expect("256 accepted").len(), 256);
    let mut over = ok.clone();
    over.push(moved(Some(2), vec![pa(), pb()], true));
    assert_eq!(project_events(&over), Err(ProjectionError::TooManyEvents));
    // 257 candidates that would all be omitted are still refused.
    let hidden: Vec<EventCandidate> = (0..cap + 1).map(|_| moved(None, vec![], false)).collect();
    assert_eq!(project_events(&hidden), Err(ProjectionError::TooManyEvents));

    let resolved = |outcome: &str, actor: Option<u64>| {
        EventCandidate::V3(V3Candidate::V2(V2Candidate::ActionResolved {
            actor: actor.map(id),
            target: Some(id(2)),
            ability: Some(u(7)),
            outcome: Some(outcome.to_owned()),
            damage: Some(3),
        }))
    };
    // Control: a valid outcome projects.
    assert_eq!(
        project_events(&[resolved("success", Some(1))])
            .expect("ok")
            .len(),
        1
    );
    assert_eq!(
        project_events(&[resolved("bogus", Some(1))]),
        Err(ProjectionError::InvalidOutcome)
    );
    // The error fails the whole call, valid Moved candidates included.
    assert_eq!(
        project_events(&[
            moved(Some(2), vec![pa(), pb()], true),
            resolved("bogus", Some(1)),
        ]),
        Err(ProjectionError::InvalidOutcome)
    );
    // Not disclosed: omitted without inspection, so no error.
    assert_eq!(
        project_events(&[resolved("bogus", None)]).expect("omitted"),
        vec![]
    );
}
