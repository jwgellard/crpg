//! T140c two-keys contract tests: `facts_v3` per-viewer fact filter.
//!
//! Written from `tasks/T140.md` (Interface, Behaviour C-1..C-9, Errors, Tests
//! C1..C15) and the public `crpg-net` API alone. Expected values are literals
//! or come from the test-local `expect_image` oracle, never from running the
//! filter. Every call goes through [`Pair`], which also drives a twin
//! `InterestView` with the same states: the view's ops, `enters` and `cursor`
//! must equal the twin's, and after every `Err` the view and the fact view
//! must equal their pre-call clones.

use std::error::Error;

use crpg_core::{EntityId, GenerationalArena, Ulid};
use crpg_net::codec_v3 as c3;
use crpg_net::facts_v3::{
    clock_op, host_facts_op, project_page, EncounterGrants, EncounterInput, FactContext, FactError,
    FactMask, FactRow, FactView, HostGrants, HostInput, OrderInput, PageV3, MAX_FACT_PAGE_OPS,
    MAX_PAGE_OPS_V3,
};
use crpg_net::interest::{
    EntityFact, EntityKey, FieldMask, InterestError, InterestSet, InterestView, TurnState,
    ViewCursor, ViewerContext,
};
use crpg_net::protocol_transfer::HealthFact;
use crpg_net::protocol_v3::{
    AutoPauseToggles, AutoPauseTrigger, ClockFact, DeltaFrame, DeltaOp, EffectFact, EncounterFacts,
    EncounterMode, FactSet, HostFacts, NetId, OrderFact, PauseReason, PauseRights, PoolFact,
    SessionEpoch, StatFact, StatValue, TimelineEntry,
};

const RT: EncounterMode = EncounterMode::RealTime;
const TB: EncounterMode = EncounterMode::TurnBased;
const CLOCK: ClockFact = ClockFact {
    round_ticks: 120,
    elapsed: 240,
};
const ALL: EncounterGrants = EncounterGrants {
    mode: true,
    pending_mode: true,
    clock: true,
    timeline: true,
};
const CURSOR1: ViewCursor = ViewCursor {
    tick: 7,
    revision: 1,
};

fn net(raw: u64) -> NetId {
    NetId::new(raw).expect("nonzero fixture id")
}
fn ulid(n: u128) -> Ulid {
    Ulid::from_u128(n)
}
fn epoch(b: u8) -> SessionEpoch {
    SessionEpoch([b; 16])
}
fn pool(n: u128, current: u32, max: u32) -> PoolFact {
    PoolFact {
        pool: ulid(n),
        current,
        max,
    }
}
fn effect(n: u128, expires_round: u64) -> EffectFact {
    EffectFact {
        effect: ulid(n),
        expires_round,
    }
}
fn istat(name: &str, v: i32) -> StatFact {
    StatFact {
        name: name.to_string(),
        value: StatValue::Int(v),
    }
}
fn mask(p: bool, e: bool, o: bool, s: bool, t: bool) -> FactMask {
    FactMask {
        pools: p,
        effects: e,
        order: o,
        stats: s,
        tags: t,
    }
}
fn row(k: EntityKey, fields: FactMask) -> FactRow {
    FactRow {
        key: k,
        fields,
        pools: vec![],
        effects: vec![],
        order: None,
        stats: vec![],
        tags: vec![],
    }
}
fn order_in(target: EntityKey, repeat: bool) -> Option<OrderInput> {
    Some(OrderInput {
        ability: ulid(7),
        target,
        repeat,
    })
}
fn ids(n: usize) -> Vec<EntityId> {
    let mut arena = GenerationalArena::<()>::new();
    (0..n).map(|_| arena.insert(())).collect()
}

// Expected-op constructors.
fn enter(n: u64) -> DeltaOp {
    DeltaOp::EntityEnter { entity: net(n) }
}
fn enc(
    mode: Option<EncounterMode>,
    pending_mode: Option<Option<EncounterMode>>,
    clock: Option<Option<ClockFact>>,
) -> DeltaOp {
    DeltaOp::Encounter(EncounterFacts {
        mode,
        pending_mode,
        clock,
    })
}
fn enc_full() -> DeltaOp {
    enc(Some(RT), Some(None), Some(Some(CLOCK)))
}
fn tl(entries: &[(i32, u64)]) -> DeltaOp {
    DeltaOp::Timeline {
        entries: Some(
            entries
                .iter()
                .map(|&(key, e)| TimelineEntry {
                    key,
                    entity: net(e),
                })
                .collect(),
        ),
    }
}
fn tl_none() -> DeltaOp {
    DeltaOp::Timeline { entries: None }
}
fn ef(n: u64, facts: FactSet) -> DeltaOp {
    DeltaOp::EntityFacts {
        entity: net(n),
        facts,
    }
}
fn k2_img() -> FactSet {
    FactSet {
        pools: Some(vec![pool(1, 2, 3)]),
        stats: Some(vec![istat("might", 4)]),
        ..FactSet::default()
    }
}
fn c2_tail() -> Vec<DeltaOp> {
    vec![enc_full(), tl(&[(-1, 1), (-1, 2), (5, 3)]), ef(2, k2_img())]
}
fn page_bytes(ops: &[DeltaOp]) -> Vec<u8> {
    c3::encode_delta(&DeltaFrame {
        lane: 0,
        server_tick: 100,
        event_seq: 1,
        ops: ops.to_vec(),
    })
    .expect("small page encodes")
}

/// Oracle for the C-4 row image of `r`; `order_net` is the target's `NetId`.
fn expect_image(r: &FactRow, order_net: Option<u64>) -> FactSet {
    let m = r.fields;
    let order = if !m.order {
        None
    } else {
        match &r.order {
            None => Some(None),
            Some(o) => order_net.map(|n| {
                Some(OrderFact {
                    ability: o.ability,
                    target: net(n),
                    repeat: o.repeat,
                })
            }),
        }
    };
    FactSet {
        pools: m.pools.then(|| r.pools.clone()),
        effects: m.effects.then(|| r.effects.clone()),
        order,
        stats: m.stats.then(|| r.stats.clone()),
        tags: m.tags.then(|| r.tags.clone()),
    }
}

/// One scenario input; the spec's fixture unless changed.
#[derive(Clone)]
struct Fx {
    ids: Vec<EntityId>,
    area: u128,
    epoch: SessionEpoch,
    facts: Vec<EntityFact>,
    despawned: Vec<EntityKey>,
    turn: Option<TurnState>,
    rows: Vec<FactRow>,
    enc_on: bool,
    enc_area: Ulid,
    mode: EncounterMode,
    pending: Option<EncounterMode>,
    clock: Option<ClockFact>,
    timeline: Vec<(i32, EntityKey)>,
    grants: EncounterGrants,
    resend: bool,
}

fn efact(k: EntityKey, presence: bool, health: Option<HealthFact>) -> EntityFact {
    EntityFact {
        key: k,
        fields: FieldMask {
            presence,
            health: health.is_some(),
        },
        health,
        resend_health: false,
    }
}

impl Fx {
    fn sized(area: u128, n: usize) -> Fx {
        let mut fx = Fx {
            ids: ids(n),
            area,
            epoch: epoch(0),
            facts: vec![],
            despawned: vec![],
            turn: None,
            rows: vec![],
            enc_on: true,
            enc_area: ulid(area),
            mode: RT,
            pending: None,
            clock: Some(CLOCK),
            timeline: vec![],
            grants: ALL,
            resend: false,
        };
        let (k1, k2, k3) = (fx.k(1), fx.k(2), fx.k(3));
        let alive = HealthFact {
            health: 10,
            max_health: 10,
            dead: false,
        };
        fx.facts = vec![
            efact(k1, true, Some(alive)),
            efact(k2, true, None),
            efact(k3, true, None),
        ];
        let mut r2 = row(k2, mask(true, false, false, true, false));
        r2.pools = vec![pool(1, 2, 3)];
        r2.stats = vec![istat("might", 4)];
        let mut r3 = row(k3, FactMask::default());
        r3.pools = vec![pool(2, 1, 1)];
        r3.effects = vec![effect(3, 9)];
        r3.order = order_in(k1, true);
        r3.stats = vec![istat("grit", 1)];
        r3.tags = vec!["t".to_string()];
        fx.rows = vec![r2, r3];
        fx.timeline = vec![(5, k3), (-1, k2), (-1, k1)];
        fx
    }
    fn new() -> Fx {
        Fx::sized(10, 4)
    }
    fn k(&self, n: usize) -> EntityKey {
        EntityKey {
            area: ulid(self.area),
            entity: self.ids[n - 1],
        }
    }
    fn row_mut(&mut self, n: usize) -> &mut FactRow {
        let k = self.k(n);
        self.rows.iter_mut().find(|r| r.key == k).expect("row")
    }
    fn set_presence(&mut self, n: usize, on: bool) {
        let k = self.k(n);
        match self.facts.iter_mut().find(|f| f.key == k) {
            Some(f) => f.fields.presence = on,
            None => self.facts.push(efact(k, on, None)),
        }
    }
    fn state(&self) -> ViewerContext<'_> {
        ViewerContext {
            epoch: self.epoch,
            tick: 7,
            interest: InterestSet {
                area: ulid(self.area),
            },
            facts: &self.facts,
            despawned: &self.despawned,
            turn: self.turn,
            turn_granted: self.turn.is_some(),
            resend_turn: false,
        }
    }
    fn run(&self, view: &mut InterestView, fv: &mut FactView) -> Result<PageV3, FactError> {
        let encounter = self.enc_on.then_some(EncounterInput {
            area: self.enc_area,
            mode: self.mode,
            pending_mode: self.pending,
            clock: self.clock,
            timeline: &self.timeline,
        });
        let context = FactContext {
            rows: &self.rows,
            encounter,
            grants: self.grants,
            resend: self.resend,
        };
        project_page(view, fv, &self.state(), &context)
    }
}

/// A view, a fact view and a twin view that sees the same successful states.
struct Pair {
    view: InterestView,
    fv: FactView,
    twin: InterestView,
}

impl Pair {
    fn new() -> Pair {
        Pair::with(0, 0)
    }
    fn with(view_epoch: u8, fv_epoch: u8) -> Pair {
        Pair {
            view: InterestView::new(epoch(view_epoch)),
            fv: FactView::new(epoch(fv_epoch)),
            twin: InterestView::new(epoch(view_epoch)),
        }
    }
    fn run(&mut self, fx: &Fx) -> Result<(PageV3, usize), FactError> {
        let (v0, f0) = (self.view.clone(), self.fv.clone());
        match fx.run(&mut self.view, &mut self.fv) {
            Err(e) => {
                assert_eq!(self.view, v0, "view changed by {e:?}");
                assert_eq!(self.fv, f0, "fact view changed by {e:?}");
                Err(e)
            }
            Ok(page) => {
                let t = self.twin.project_delta(&fx.state()).expect("twin accepts");
                let n = t.ops.len();
                let want: Vec<DeltaOp> = t.ops.into_iter().map(DeltaOp::from).collect();
                assert!(page.ops.len() >= n);
                assert_eq!(page.ops[..n], want[..], "view ops come first, converted");
                assert_eq!(page.enters, t.enters);
                assert_eq!(page.cursor, t.cursor);
                assert_eq!(self.view, self.twin);
                assert!(page.ops.len() <= MAX_PAGE_OPS_V3);
                Ok((page, n))
            }
        }
    }
    fn err(&mut self, fx: &Fx) -> FactError {
        self.run(fx).expect_err("must be rejected")
    }
    fn page(&mut self, fx: &Fx) -> PageV3 {
        self.run(fx).expect("accepted").0
    }
    /// The fact ops only (everything after the view's ops).
    fn step(&mut self, fx: &Fx) -> Vec<DeltaOp> {
        let (mut page, n) = self.run(fx).expect("accepted");
        page.ops.split_off(n)
    }
}

// ---------------------------------------------------------------------------
// C-1 / C1: checks, their order, all-or-nothing.
// ---------------------------------------------------------------------------

fn sized_lists(rows: usize, tl_len: usize) -> Fx {
    let mut fx = Fx::sized(10, 1030);
    let mut i = 4;
    while fx.rows.len() < rows {
        let k = fx.k(i);
        fx.rows.push(row(k, FactMask::default()));
        i += 1;
    }
    i = 4;
    while fx.timeline.len() < tl_len {
        let k = fx.k(i);
        fx.timeline.push((0, k));
        i += 1;
    }
    fx
}

#[test]
fn c01_epoch_mismatches_are_wrong_epoch_first() {
    // (view, facts, state) epochs; the view/facts mismatch is FactError, not Interest.
    for (v, f, s) in [
        (0, 0, 1),
        (0, 1, 0),
        (1, 0, 0),
        (1, 0, 1),
        (0, 1, 1),
        (1, 1, 0),
    ] {
        let mut fx = Fx::new();
        fx.epoch = epoch(s);
        assert_eq!(
            Pair::with(v, f).err(&fx),
            FactError::WrongEpoch,
            "{v}{f}{s}"
        );
    }
    let mut fx = Fx::new();
    fx.epoch = epoch(1);
    assert_eq!(Pair::with(1, 1).step(&fx), c2_tail());
    // Step 1 beats steps 2 and 3.
    let mut big = sized_lists(1025, 1025);
    big.rows.push(row(big.k(2), FactMask::default()));
    big.epoch = epoch(1);
    assert_eq!(Pair::with(0, 0).err(&big), FactError::WrongEpoch);
}

#[test]
fn c01_row_and_timeline_bounds() {
    let first = |fx: &Fx| Pair::new().step(fx);
    assert_eq!(first(&sized_lists(1024, 3)), c2_tail());
    assert_eq!(first(&sized_lists(2, 1024)), c2_tail());
    assert_eq!(first(&sized_lists(1024, 1024)), c2_tail());
    let over = sized_lists(1025, 3);
    assert_eq!(Pair::new().err(&over), FactError::TooManyFacts);
    let over = sized_lists(2, 1025);
    assert_eq!(Pair::new().err(&over), FactError::TooManyFacts);
    // Foreign encounter, or no timeline grant: the bound still applies.
    let mut foreign = sized_lists(2, 1025);
    foreign.enc_area = ulid(11);
    assert_eq!(Pair::new().err(&foreign), FactError::TooManyFacts);
    let mut ungranted = sized_lists(2, 1025);
    ungranted.grants.timeline = false;
    assert_eq!(Pair::new().err(&ungranted), FactError::TooManyFacts);
    // An absent encounter has no timeline to bound.
    let mut none = sized_lists(2, 1025);
    none.enc_on = false;
    assert_eq!(Pair::new().step(&none), vec![ef(2, k2_img())]);
}

#[test]
fn c01_too_many_beats_duplicate_and_duplicates_at_the_bound() {
    // 1,025 rows holding a duplicate: TooManyFacts (step 2 before 3).
    let mut fx = sized_lists(1025, 3);
    fx.rows[1024] = row(fx.k(2), FactMask::default());
    assert_eq!(Pair::new().err(&fx), FactError::TooManyFacts);
    let mut fx = sized_lists(2, 1025);
    fx.timeline[1024] = (3, fx.k(2));
    assert_eq!(Pair::new().err(&fx), FactError::TooManyFacts);
    // Rows hold a duplicate, the timeline is over: still step 2.
    let mut fx = sized_lists(2, 1025);
    fx.rows.push(row(fx.k(2), FactMask::default()));
    assert_eq!(Pair::new().err(&fx), FactError::TooManyFacts);
    // Exactly 1,024 entries with a duplicate: DuplicateFact.
    let mut fx = sized_lists(1024, 3);
    fx.rows[1023] = row(fx.k(2), FactMask::default());
    assert_eq!(Pair::new().err(&fx), FactError::DuplicateFact);
    let mut fx = sized_lists(2, 1024);
    fx.timeline[1023] = (9, fx.k(1));
    assert_eq!(Pair::new().err(&fx), FactError::DuplicateFact);
}

#[test]
fn c01_duplicate_rows_adjacent_and_not() {
    let base = Fx::new();
    let (k1, k2, k3, k4) = (base.k(1), base.k(2), base.k(3), base.k(4));
    let lists = [
        vec![k2, k2, k3],
        vec![k2, k3, k2],
        vec![k3, k2, k1, k2],
        vec![k4, k2, k4], // a key with no NetId still counts
        vec![k1, k1],
    ];
    for keys in lists {
        let mut fx = base.clone();
        fx.rows = keys
            .iter()
            .enumerate()
            .map(|(i, &k)| row(k, mask(i % 2 == 0, false, false, false, false)))
            .collect();
        assert_eq!(Pair::new().err(&fx), FactError::DuplicateFact, "{keys:?}");
    }
    let mut ok = base.clone();
    ok.rows = vec![row(k4, FactMask::default()), row(k2, FactMask::default())];
    Pair::new().step(&ok);
}

#[test]
fn c01_duplicate_timeline_entries_adjacent_and_not() {
    let base = Fx::new();
    let (k1, k2, k3, k4) = (base.k(1), base.k(2), base.k(3), base.k(4));
    let lists: [Vec<(i32, EntityKey)>; 6] = [
        vec![(5, k3), (-1, k2), (-1, k2)],         // adjacent, same key
        vec![(5, k3), (-1, k2), (4, k2)],          // adjacent, different key
        vec![(5, k3), (-1, k2), (0, k1), (7, k2)], // not adjacent
        vec![(1, k2), (2, k1), (3, k2)],           // not adjacent in any order
        vec![(3, k4), (1, k1), (2, k3), (4, k4)],  // invisible key repeated
        vec![(9, k1), (9, k1)],
    ];
    for entries in lists {
        let mut fx = base.clone();
        fx.timeline = entries.clone();
        assert_eq!(
            Pair::new().err(&fx),
            FactError::DuplicateFact,
            "{entries:?}"
        );
        // The same list is refused for a foreign encounter and without the grant.
        fx.enc_area = ulid(11);
        assert_eq!(Pair::new().err(&fx), FactError::DuplicateFact);
        fx.enc_area = ulid(10);
        fx.grants.timeline = false;
        assert_eq!(Pair::new().err(&fx), FactError::DuplicateFact);
    }
    let mut ok = base.clone();
    ok.timeline = vec![(1, k2), (1, k1), (1, k4)];
    assert_eq!(Pair::new().step(&ok)[1], tl(&[(1, 1), (1, 2)]));
}

#[test]
fn c01_fact_checks_come_before_the_interest_check() {
    // State holds a duplicate, rows are fine: Interest. Rows duplicate too: DuplicateFact.
    let mut fx = Fx::new();
    let dup = efact(fx.k(1), true, None);
    fx.facts.push(dup);
    assert_eq!(
        Pair::new().err(&fx),
        FactError::Interest(InterestError::DuplicateFact)
    );
    fx.rows.push(row(fx.k(2), FactMask::default()));
    assert_eq!(Pair::new().err(&fx), FactError::DuplicateFact);
    // Too many rows beat a failing state.
    let mut fx = sized_lists(1025, 3);
    let dup = efact(fx.k(1), true, None);
    fx.facts.push(dup);
    assert_eq!(Pair::new().err(&fx), FactError::TooManyFacts);
}

// ---------------------------------------------------------------------------
// C-6 / C2, C3: layout and memory.
// ---------------------------------------------------------------------------

#[test]
fn c02_first_page_layout() {
    let fx = Fx::new();
    let mut p = Pair::new();
    let page = p.page(&fx);
    let mut ops = vec![
        enter(1),
        enter(2),
        enter(3),
        DeltaOp::Health {
            entity: net(1),
            health: 10,
            max_health: 10,
            dead: false,
        },
    ];
    ops.extend(c2_tail());
    assert_eq!(
        page,
        PageV3 {
            ops,
            enters: 3,
            cursor: CURSOR1
        }
    );
    assert_ne!(p.fv, FactView::new(epoch(0)));
    assert_eq!(p.fv.epoch(), epoch(0));
    assert_eq!(FactView::new(epoch(3)).epoch(), epoch(3));
    assert_eq!(FactView::new(epoch(3)), FactView::new(epoch(3)));
    // Rows may come in any order.
    let mut rev = Fx::new();
    rev.rows.reverse();
    assert_eq!(Pair::new().step(&rev), c2_tail());
}

#[test]
fn c03_unchanged_page_is_empty() {
    let fx = Fx::new();
    let mut p = Pair::new();
    p.page(&fx);
    for _ in 0..2 {
        assert_eq!(
            p.page(&fx),
            PageV3 {
                ops: vec![],
                enters: 0,
                cursor: CURSOR1
            }
        );
    }
    // Input order of the timeline and of the rows does not matter.
    let mut perm = fx.clone();
    perm.timeline.reverse();
    perm.rows.reverse();
    assert_eq!(p.step(&perm), vec![]);
}

#[test]
fn c03_timeline_image_rules() {
    // Ties are ordered by EntityKey, not NetId: k1 re-enters as NetId 4.
    let mut fx = Fx::new();
    let mut p = Pair::new();
    p.page(&fx);
    fx.set_presence(1, false);
    assert_eq!(p.step(&fx), vec![tl(&[(-1, 2), (5, 3)])]);
    fx.set_presence(1, true);
    assert_eq!(p.step(&fx), vec![tl(&[(-1, 4), (-1, 2), (5, 3)])]);
    // Keys with no NetId are dropped; none left is a disclosed empty list.
    let mut fx = Fx::new();
    fx.timeline = vec![(3, fx.k(4)), (1, fx.k(1)), (2, fx.k(3))];
    fx.set_presence(3, false);
    assert_eq!(Pair::new().step(&fx)[1], tl(&[(1, 1)]));
    fx.timeline = vec![(3, fx.k(4))];
    assert_eq!(Pair::new().step(&fx)[1], tl(&[]));
    // A changed key value is a change.
    let mut fx = Fx::new();
    let mut p = Pair::new();
    p.page(&fx);
    fx.timeline[0].0 = 6;
    assert_eq!(p.step(&fx), vec![tl(&[(-1, 1), (-1, 2), (6, 3)])]);
}

#[test]
fn c06_fact_ops_follow_all_view_ops_in_order() {
    let mut fx = Fx::new();
    let mut p = Pair::new();
    p.page(&fx);
    // k4 enters with a full-mask row, k3 despawns, and the turn changes.
    fx.set_presence(4, true);
    let k4 = fx.k(4);
    let mut r4 = row(k4, mask(true, false, false, false, false));
    r4.pools = vec![pool(5, 1, 1)];
    fx.rows.push(r4);
    fx.set_presence(3, false);
    fx.despawned = vec![fx.k(3)];
    fx.turn = Some(TurnState {
        area: ulid(10),
        active: Some(fx.ids[0]),
        round: 2,
    });
    let page = p.page(&fx);
    let want = vec![
        enter(4),
        DeltaOp::Turn {
            active: Some(net(1)),
            round: 2,
        },
        DeltaOp::Despawned { entity: net(3) },
        tl(&[(-1, 1), (-1, 2)]),
        ef(
            4,
            FactSet {
                pools: Some(vec![pool(5, 1, 1)]),
                ..FactSet::default()
            },
        ),
    ];
    assert_eq!(page.ops, want);
    assert_eq!(page.enters, 1);
    assert_eq!(page.cursor.revision, 2);
}

#[test]
fn c06_entity_facts_ascend_by_net_id() {
    let mut fx = Fx::new();
    fx.grants = EncounterGrants::default();
    let k1 = fx.k(1);
    let mut r1 = row(k1, mask(true, false, false, false, false));
    r1.pools = vec![pool(9, 1, 1)];
    fx.rows.push(r1);
    fx.rows.reverse(); // rows arrive in any order
    let pools9 = FactSet {
        pools: Some(vec![pool(9, 1, 1)]),
        ..FactSet::default()
    };
    let mut p = Pair::new();
    assert_eq!(p.step(&fx), vec![ef(1, pools9.clone()), ef(2, k2_img())]);
    // k1 leaves and re-enters as NetId 4 while k2 and k3 change: 2, 3, 4.
    fx.set_presence(1, false);
    assert_eq!(p.step(&fx), vec![]);
    fx.set_presence(1, true);
    fx.row_mut(2).pools[0].current = 0;
    fx.row_mut(3).fields.pools = true;
    let k2_new = FactSet {
        pools: Some(vec![pool(1, 0, 3)]),
        stats: Some(vec![istat("might", 4)]),
        ..FactSet::default()
    };
    let k3_new = FactSet {
        pools: Some(vec![pool(2, 1, 1)]),
        ..FactSet::default()
    };
    assert_eq!(
        p.step(&fx),
        vec![ef(2, k2_new), ef(3, k3_new), ef(4, pools9)]
    );
}

// ---------------------------------------------------------------------------
// C-2 / C-3 grants.
// ---------------------------------------------------------------------------

#[test]
fn c02_every_grant_combination() {
    for bits in 0..16u8 {
        for pending in [None, Some(TB)] {
            let g = EncounterGrants {
                mode: bits & 1 != 0,
                pending_mode: bits & 2 != 0,
                clock: bits & 4 != 0,
                timeline: bits & 8 != 0,
            };
            let mut fx = Fx::new();
            fx.grants = g;
            fx.pending = pending;
            let mut want = vec![];
            if g.mode || g.pending_mode || g.clock {
                want.push(enc(
                    g.mode.then_some(RT),
                    g.pending_mode.then_some(pending),
                    g.clock.then_some(Some(CLOCK)),
                ));
            }
            if g.timeline {
                want.push(tl(&[(-1, 1), (-1, 2), (5, 3)]));
            }
            want.push(ef(2, k2_img()));
            assert_eq!(Pair::new().step(&fx), want, "{g:?} {pending:?}");
        }
    }
    // Absent clock with the grant: disclosed "none". TurnBased mode.
    let mut fx = Fx::new();
    fx.clock = None;
    fx.mode = TB;
    assert_eq!(
        Pair::new().step(&fx)[0],
        enc(Some(TB), Some(None), Some(None))
    );
}

#[test]
fn c11_foreign_area_encounter() {
    let mut fx = Fx::new();
    let mut p = Pair::new();
    assert_eq!(p.step(&fx), c2_tail());
    fx.enc_area = ulid(11);
    assert_eq!(p.step(&fx), vec![enc(None, None, None), tl_none()]);
    assert_eq!(p.step(&fx), vec![]);
    fx.clock = None;
    fx.pending = Some(TB);
    assert_eq!(p.step(&fx), vec![]);
    fx.resend = true;
    assert_eq!(p.step(&fx), vec![ef(2, k2_img())]);
    fx.resend = false;
    fx.enc_area = ulid(10);
    assert_eq!(
        p.step(&fx),
        vec![
            enc(Some(RT), Some(Some(TB)), Some(None)),
            tl(&[(-1, 1), (-1, 2), (5, 3)])
        ]
    );
    // No encounter at all is the same as a foreign one.
    fx.enc_on = false;
    assert_eq!(p.step(&fx), vec![enc(None, None, None), tl_none()]);
    assert_eq!(p.step(&fx), vec![]);
    // A foreign encounter from the start discloses nothing.
    let mut first = Fx::new();
    first.enc_area = ulid(11);
    assert_eq!(Pair::new().step(&first), vec![ef(2, k2_img())]);
}

#[test]
fn c11_state_area_is_the_comparison() {
    // Keys, state and encounter all in area 11: disclosed.
    let mut fx = Fx::sized(11, 4);
    assert_eq!(Pair::new().step(&fx), c2_tail());
    // Encounter in the old area 10 against state area 11: withheld.
    fx.enc_area = ulid(10);
    assert_eq!(Pair::new().step(&fx), vec![ef(2, k2_img())]);
}

// ---------------------------------------------------------------------------
// C-4 / C4: row image, change, revocation.
// ---------------------------------------------------------------------------

#[test]
fn c04_change_and_revocation() {
    let mut fx = Fx::new();
    let mut p = Pair::new();
    p.page(&fx);
    fx.row_mut(2).pools[0].current = 1;
    let want = FactSet {
        pools: Some(vec![pool(1, 1, 3)]),
        stats: Some(vec![istat("might", 4)]),
        ..FactSet::default()
    };
    assert_eq!(p.step(&fx), vec![ef(2, want)]);
    fx.row_mut(2).fields.pools = false;
    let want = FactSet {
        stats: Some(vec![istat("might", 4)]),
        ..FactSet::default()
    };
    assert_eq!(p.step(&fx), vec![ef(2, want)]);
    fx.row_mut(2).fields = FactMask::default();
    assert_eq!(p.step(&fx), vec![ef(2, FactSet::default())]);
    assert_eq!(p.step(&fx), vec![]);
    // Hidden values moving do not emit; the grant coming back does.
    fx.row_mut(2).pools[0].current = 0;
    assert_eq!(p.step(&fx), vec![]);
    fx.row_mut(2).fields.pools = true;
    let want = FactSet {
        pools: Some(vec![pool(1, 0, 3)]),
        ..FactSet::default()
    };
    assert_eq!(p.step(&fx), vec![ef(2, want)]);
}

#[test]
fn c04_each_family_change_emits_the_full_image() {
    let mut fx = Fx::new();
    let k1 = fx.k(1);
    let r = fx.row_mut(2);
    r.fields = mask(true, true, true, true, true);
    r.pools = vec![pool(1, 2, 3)];
    r.effects = vec![effect(2, 5)];
    r.order = order_in(k1, false);
    r.stats = vec![istat("might", 4)];
    r.tags = vec!["a".to_string()];
    let mut p = Pair::new();
    p.page(&fx);
    let edits: [fn(&mut FactRow); 7] = [
        |r| r.pools[0].max = 4,
        |r| r.effects[0].expires_round = 6,
        |r| r.effects.push(effect(8, 1)),
        |r| r.order.as_mut().unwrap().repeat = true,
        |r| r.stats[0].value = StatValue::Fixed(4),
        |r| r.stats[0].name = "might2".to_string(),
        |r| r.tags[0] = "b".to_string(),
    ];
    for edit in edits {
        edit(fx.row_mut(2));
        let want = expect_image(fx.row_mut(2), Some(1));
        assert_eq!(p.step(&fx), vec![ef(2, want)]);
    }
}

#[test]
fn c04_every_mask_renders_only_granted_families() {
    for bits in 0..32u8 {
        let m = mask(
            bits & 1 != 0,
            bits & 2 != 0,
            bits & 4 != 0,
            bits & 8 != 0,
            bits & 16 != 0,
        );
        let mut fx = Fx::new();
        fx.grants = EncounterGrants::default();
        let k1 = fx.k(1);
        let r = fx.row_mut(2);
        r.fields = m;
        r.pools = vec![pool(1, 2, 3)];
        r.effects = vec![effect(2, 5)];
        r.order = order_in(k1, true);
        r.stats = vec![istat("might", 4)];
        r.tags = vec!["a".to_string()];
        let want = expect_image(r, Some(1));
        let tail = Pair::new().step(&fx);
        if bits == 0 {
            assert_eq!(tail, vec![], "default mask");
        } else {
            assert_eq!(tail, vec![ef(2, want)], "{m:?}");
        }
    }
    // A granted but empty family is a disclosed empty list.
    let mut fx = Fx::new();
    fx.grants = EncounterGrants::default();
    fx.row_mut(2).pools.clear();
    fx.row_mut(2).stats.clear();
    let want = FactSet {
        pools: Some(vec![]),
        stats: Some(vec![]),
        ..FactSet::default()
    };
    assert_eq!(Pair::new().step(&fx), vec![ef(2, want)]);
}

#[test]
fn c04_rows_for_invisible_keys_are_ignored() {
    let mut fx = Fx::new();
    let k4 = fx.k(4);
    let mut r4 = row(k4, mask(true, false, false, false, false));
    r4.pools = vec![pool(6, 1, 2)];
    fx.rows.push(r4);
    let mut p = Pair::new();
    assert_eq!(p.step(&fx), c2_tail());
    fx.set_presence(4, true);
    let want = FactSet {
        pools: Some(vec![pool(6, 1, 2)]),
        ..FactSet::default()
    };
    // Entering now: its NetId is 4 and the row applies at once.
    assert_eq!(p.step(&fx), vec![ef(4, want)]);
}

// ---------------------------------------------------------------------------
// C-8 / C8: orders.
// ---------------------------------------------------------------------------

fn order_fx(order: Option<OrderInput>, m: FactMask) -> Fx {
    let mut fx = Fx::new();
    fx.grants = EncounterGrants::default();
    let r = fx.row_mut(2);
    r.fields = m;
    r.order = order;
    fx
}

#[test]
fn c08_order_rules() {
    let only_pools_order_none = || FactSet {
        pools: Some(vec![pool(1, 2, 3)]),
        order: None,
        ..FactSet::default()
    };
    let with_order = mask(true, false, true, false, false);
    let k1 = Fx::new().k(1);
    let only = |order: Option<Option<OrderFact>>| {
        vec![ef(
            2,
            FactSet {
                pools: Some(vec![pool(1, 2, 3)]),
                order,
                ..FactSet::default()
            },
        )]
    };
    let step = |fx: &Fx| Pair::new().step(fx);
    assert_eq!(step(&order_fx(None, with_order)), only(Some(None)));
    let named = |repeat| {
        Some(Some(OrderFact {
            ability: ulid(7),
            target: net(1),
            repeat,
        }))
    };
    assert_eq!(
        step(&order_fx(order_in(k1, false), with_order)),
        only(named(false))
    );
    assert_eq!(
        step(&order_fx(order_in(k1, true), with_order)),
        only(named(true))
    );
    // Target not in the state: the family is withheld.
    let k4 = Fx::new().k(4);
    assert_eq!(step(&order_fx(order_in(k4, true), with_order)), only(None));
    // Target in the state but not visible (no presence): withheld.
    let mut hidden = order_fx(order_in(k1, true), with_order);
    hidden.set_presence(1, false);
    // (k1 has no NetId, so k2 is the first to enter and is NetId 1.)
    assert_eq!(step(&hidden), vec![ef(1, only_pools_order_none())]);
    // The default mask never discloses an order, whatever the row holds.
    assert!(!FactMask::default().order);
    let plain = mask(true, false, false, false, false);
    assert_eq!(
        step(&order_fx(order_in(k1, true), plain)),
        vec![ef(
            2,
            FactSet {
                pools: Some(vec![pool(1, 2, 3)]),
                ..FactSet::default()
            }
        )]
    );
    let mut none = Fx::new();
    none.grants = EncounterGrants::default();
    none.rows = vec![none.rows[1].clone()];
    assert_eq!(Pair::new().step(&none), vec![]);
}

#[test]
fn c08_order_follows_target_visibility() {
    let with_order = mask(true, false, true, false, false);
    let mut fx = order_fx(None, with_order);
    let k4 = fx.k(4);
    fx.row_mut(2).order = order_in(k4, false);
    let mut p = Pair::new();
    p.page(&fx);
    fx.set_presence(4, true);
    let named = FactSet {
        pools: Some(vec![pool(1, 2, 3)]),
        order: Some(Some(OrderFact {
            ability: ulid(7),
            target: net(4),
            repeat: false,
        })),
        ..FactSet::default()
    };
    assert_eq!(p.step(&fx), vec![ef(2, named)]);
    fx.set_presence(4, false);
    let gone = FactSet {
        pools: Some(vec![pool(1, 2, 3)]),
        order: None,
        ..FactSet::default()
    };
    assert_eq!(p.step(&fx), vec![ef(2, gone)]);
}

// ---------------------------------------------------------------------------
// C-4 / C9: over-limit families.
// ---------------------------------------------------------------------------

fn loaded(fx: &mut Fx, n: [usize; 4]) {
    let r = fx.row_mut(2);
    r.fields = mask(true, true, false, true, true);
    r.pools = (0..n[0]).map(|i| pool(i as u128 + 1, 1, 2)).collect();
    r.effects = (0..n[1]).map(|i| effect(i as u128 + 1, 3)).collect();
    r.stats = (0..n[2]).map(|i| istat(&format!("s{i}"), 1)).collect();
    r.tags = (0..n[3]).map(|i| format!("t{i}")).collect();
}

#[test]
fn c09_overlimit_families_are_withheld_not_truncated() {
    let limits = [16, 16, 32, 8];
    let mut fx = Fx::new();
    loaded(&mut fx, limits);
    let want = expect_image(fx.row_mut(2), None);
    assert_eq!(want.pools.as_ref().map(Vec::len), Some(16));
    assert_eq!(Pair::new().step(&fx)[2], ef(2, want));
    for family in 0..4 {
        let mut n = limits;
        n[family] += 1;
        let mut fx = Fx::new();
        loaded(&mut fx, n);
        let mut want = expect_image(fx.row_mut(2), None);
        match family {
            0 => want.pools = None,
            1 => want.effects = None,
            2 => want.stats = None,
            _ => want.tags = None,
        }
        let tail = Pair::new().step(&fx);
        assert_eq!(
            tail,
            vec![enc_full(), tl(&[(-1, 1), (-1, 2), (5, 3)]), ef(2, want)]
        );
    }
}

#[test]
fn c09_names_and_pool_values_are_checked() {
    let long33 = format!("{}a", "\u{e9}".repeat(16)); // 33 bytes
    let ok32 = "\u{e9}".repeat(16); // 32 bytes
    assert_eq!((ok32.len(), long33.len()), (32, 33));
    let run = |stat: &str, tag: &str, pool_ok: bool| {
        let mut fx = Fx::new();
        loaded(&mut fx, [1, 1, 1, 1]);
        fx.row_mut(2).stats[0].name = stat.to_string();
        fx.row_mut(2).tags[0] = tag.to_string();
        fx.row_mut(2).pools[0] = if pool_ok {
            pool(1, 3, 3)
        } else {
            pool(1, 4, 3)
        };
        let want = expect_image(fx.row_mut(2), None);
        let got = Pair::new().step(&fx);
        (want, got[2].clone())
    };
    let (want, got) = run(&ok32, &ok32, true);
    assert_eq!(got, ef(2, want));
    let (mut want, got) = run(&long33, "t", true);
    want.stats = None;
    assert_eq!(got, ef(2, want));
    let (mut want, got) = run("s", &long33, true);
    want.tags = None;
    assert_eq!(got, ef(2, want));
    let (mut want, got) = run("", "t", true);
    want.stats = None;
    assert_eq!(got, ef(2, want));
    let (mut want, got) = run("s", "", true);
    want.tags = None;
    assert_eq!(got, ef(2, want));
    let (mut want, got) = run("s", "t", false);
    want.pools = None;
    assert_eq!(got, ef(2, want));
}

#[test]
fn c09_overlimit_family_does_not_change_other_rows_or_fail() {
    let mut fx = Fx::new();
    fx.row_mut(2).pools = (0..17).map(|i| pool(i + 1, 1, 1)).collect();
    let want = FactSet {
        stats: Some(vec![istat("might", 4)]),
        ..FactSet::default()
    };
    let mut p = Pair::new();
    let mut expected = c2_tail();
    expected[2] = ef(2, want.clone());
    assert_eq!(p.step(&fx), expected);
    // Back under the limit, the family is disclosed.
    fx.row_mut(2).pools.truncate(16);
    let full = FactSet {
        pools: Some(fx.row_mut(2).pools.clone()),
        ..want
    };
    assert_eq!(p.step(&fx), vec![ef(2, full)]);
}

// ---------------------------------------------------------------------------
// C-6 / C5: resend.
// ---------------------------------------------------------------------------

#[test]
fn c05_resend() {
    let mut fx = Fx::new();
    let mut p = Pair::new();
    p.page(&fx);
    fx.resend = true;
    assert_eq!(p.step(&fx), c2_tail());
    fx.resend = false;
    assert_eq!(p.step(&fx), vec![]);
    // On a first page, resend does not duplicate anything.
    let mut first = Fx::new();
    first.resend = true;
    assert_eq!(Pair::new().step(&first), c2_tail());
}

#[test]
fn c05_resend_skips_withheld_images() {
    let mut fx = Fx::new();
    fx.resend = true;
    let mut p = Pair::new();
    p.page(&fx);
    // Only the mode is granted: the timeline and the rows' images are not sent.
    fx.grants = EncounterGrants {
        mode: true,
        ..EncounterGrants::default()
    };
    fx.row_mut(2).fields = FactMask::default();
    let mut p2 = Pair::new();
    p2.page(&fx);
    assert_eq!(p2.step(&fx), vec![enc(Some(RT), None, None)]);
    // Nothing granted, or a foreign encounter: no resend at all.
    fx.grants = EncounterGrants::default();
    assert_eq!(p2.step(&fx), vec![enc(None, None, None)]);
    assert_eq!(p2.step(&fx), vec![]);
    let mut foreign = Fx::new();
    foreign.enc_area = ulid(11);
    foreign.resend = true;
    let mut p3 = Pair::new();
    assert_eq!(p3.step(&foreign), vec![ef(2, k2_img())]);
    assert_eq!(p3.step(&foreign), vec![ef(2, k2_img())]);
    // The timeline alone, as a disclosed empty list, is re-sent.
    let mut empty = Fx::new();
    empty.grants = EncounterGrants {
        timeline: true,
        ..EncounterGrants::default()
    };
    empty.timeline = vec![(3, empty.k(4))];
    empty.rows.clear();
    empty.resend = true;
    let mut p4 = Pair::new();
    assert_eq!(p4.step(&empty), vec![tl(&[])]);
    assert_eq!(p4.step(&empty), vec![tl(&[])]);
    empty.resend = false;
    assert_eq!(p4.step(&empty), vec![]);
    // Only the pending-mode and clock grants: those fields only.
    let mut some = Fx::new();
    some.grants = EncounterGrants {
        pending_mode: true,
        clock: true,
        ..EncounterGrants::default()
    };
    some.rows.clear();
    some.resend = true;
    let mut p5 = Pair::new();
    let want = enc(None, Some(None), Some(Some(CLOCK)));
    assert_eq!(p5.step(&some), vec![want.clone()]);
    assert_eq!(p5.step(&some), vec![want]);
}

// ---------------------------------------------------------------------------
// C-7 / C6, C7: no grant, no op.
// ---------------------------------------------------------------------------

#[test]
fn c06_no_grant_viewer() {
    let mut fx = Fx::new();
    fx.grants = EncounterGrants::default();
    fx.row_mut(2).order = order_in(fx.k(1), true);
    fx.row_mut(2).fields = FactMask::default();
    let mut p = Pair::new();
    let (page, n) = p.run(&fx).expect("accepted");
    assert_eq!(page.ops.len(), n);
    assert_eq!(page.ops.len(), 4);
    let edits: [fn(&mut Fx); 8] = [
        |f| f.mode = TB,
        |f| f.pending = Some(TB),
        |f| f.clock = None,
        |f| {
            f.clock = Some(ClockFact {
                round_ticks: 1,
                elapsed: 2,
            })
        },
        |f| f.timeline.reverse(),
        |f| f.timeline.clear(),
        |f| f.row_mut(3).pools.push(pool(9, 1, 1)),
        |f| f.enc_area = ulid(11),
    ];
    for edit in edits {
        edit(&mut fx);
        assert_eq!(p.step(&fx), vec![]);
    }
    fx.enc_on = false;
    fx.row_mut(2).pools.clear();
    assert_eq!(p.step(&fx), vec![]);
}

#[test]
fn c07_hidden_inputs_do_not_move_bytes() {
    let mut base = Fx::new();
    base.grants = EncounterGrants {
        mode: true,
        clock: true,
        ..EncounterGrants::default()
    };
    let bytes = |fx: &Fx| page_bytes(&Pair::new().page(fx).ops);
    let want = bytes(&base);
    let edits: [fn(&mut Fx); 6] = [
        |f| f.pending = Some(TB),
        |f| f.timeline = vec![(1, f.k(2))],
        |f| f.timeline.reverse(),
        |f| f.row_mut(3).pools = vec![pool(9, 9, 9)],
        |f| f.row_mut(3).tags = vec!["x".to_string()],
        |f| f.row_mut(2).effects = vec![effect(4, 4)], // effects not in its mask
    ];
    for edit in edits {
        let mut fx = base.clone();
        edit(&mut fx);
        assert_eq!(bytes(&fx), want);
    }
    let mut ungranted = base.clone();
    ungranted.row_mut(2).order = order_in(ungranted.k(1), true);
    assert_eq!(bytes(&ungranted), want);
    // The opposite direction: the same edits move bytes once granted.
    let mut granted = base.clone();
    granted.grants = ALL;
    let granted_bytes = bytes(&granted);
    let mut pending = granted.clone();
    pending.pending = Some(TB);
    assert_ne!(bytes(&pending), granted_bytes);
    let mut timeline = granted.clone();
    timeline.timeline = vec![(1, timeline.k(2))];
    assert_ne!(bytes(&timeline), granted_bytes);
    // Later pages: the second call after hidden edits is empty bytes-wise too.
    let mut p = Pair::new();
    p.page(&base);
    let mut edited = base.clone();
    edited.pending = Some(TB);
    edited.timeline.clear();
    assert_eq!(page_bytes(&p.page(&edited).ops), page_bytes(&[]));
}

// ---------------------------------------------------------------------------
// C-5 / C10, C15: memory.
// ---------------------------------------------------------------------------

#[test]
fn c10_reentry_new_id() {
    let mut fx = Fx::new();
    let mut p = Pair::new();
    p.page(&fx);
    fx.set_presence(2, false);
    let (page, n) = p.run(&fx).expect("accepted");
    assert_eq!(page.ops[..n], [DeltaOp::EntityLeave { entity: net(2) }]);
    assert_eq!(page.ops[n..], [tl(&[(-1, 1), (5, 3)])]);
    fx.set_presence(2, true);
    let (page, n) = p.run(&fx).expect("accepted");
    assert_eq!(page.ops[..n], [enter(4)]);
    assert_eq!(
        page.ops[n..],
        [tl(&[(-1, 1), (-1, 4), (5, 3)]), ef(4, k2_img())]
    );
    // Without the timeline grant the leave carries no fact op at all.
    let mut fx = Fx::new();
    fx.grants.timeline = false;
    let mut p = Pair::new();
    p.page(&fx);
    fx.set_presence(2, false);
    assert_eq!(p.step(&fx), vec![]);
    fx.set_presence(2, true);
    assert_eq!(p.step(&fx), vec![ef(4, k2_img())]);
    // A second leave and return gets a third id, again with the image.
    fx.set_presence(2, false);
    assert_eq!(p.step(&fx), vec![]);
    fx.set_presence(2, true);
    assert_eq!(p.step(&fx), vec![ef(5, k2_img())]);
}

#[test]
fn c15_rowless_key() {
    let mut fx = Fx::new();
    let mut p = Pair::new();
    let first = p.step(&fx);
    assert!(!first
        .iter()
        .any(|op| matches!(op, DeltaOp::EntityFacts { entity, .. } if entity.get() == 1)));
    let k2 = fx.k(2);
    let r2 = fx.rows[0].clone();
    fx.rows.retain(|r| r.key != k2);
    assert_eq!(p.step(&fx), vec![ef(2, FactSet::default())]);
    assert_eq!(p.step(&fx), vec![]);
    // k3's image is already all-withheld: dropping its row emits nothing.
    let k3 = fx.k(3);
    fx.rows.retain(|r| r.key != k3);
    assert_eq!(p.step(&fx), vec![]);
    // The row coming back re-discloses.
    fx.rows.push(r2);
    assert_eq!(p.step(&fx), vec![ef(2, k2_img())]);
}

#[test]
fn c15_invalid_clock_is_withheld_not_refused() {
    let invalid = [
        ClockFact {
            round_ticks: 0,
            elapsed: 240,
        },
        ClockFact {
            round_ticks: 120,
            elapsed: 2_147_483_648,
        },
        ClockFact {
            round_ticks: 0,
            elapsed: 2_147_483_648,
        },
    ];
    for clock in invalid {
        let mut fx = Fx::new();
        fx.clock = Some(clock);
        let mut p = Pair::new();
        let mut want = c2_tail();
        want[0] = enc(Some(RT), Some(None), None);
        assert_eq!(p.step(&fx), want, "{clock:?}");
        assert_eq!(p.step(&fx), vec![]);
        // Becoming valid is a change; becoming invalid again revokes.
        fx.clock = Some(CLOCK);
        assert_eq!(p.step(&fx), vec![enc_full()]);
        fx.clock = Some(clock);
        assert_eq!(p.step(&fx), vec![enc(Some(RT), Some(None), None)]);
    }
    for clock in [
        ClockFact {
            round_ticks: 1,
            elapsed: 0,
        },
        ClockFact {
            round_ticks: u32::MAX,
            elapsed: 2_147_483_647,
        },
    ] {
        let mut fx = Fx::new();
        fx.clock = Some(clock);
        assert_eq!(
            Pair::new().step(&fx)[0],
            enc(Some(RT), Some(None), Some(Some(clock)))
        );
    }
}

// ---------------------------------------------------------------------------
// C12: page bound.
// ---------------------------------------------------------------------------

#[test]
fn c12_page_bound() {
    assert_eq!(MAX_FACT_PAGE_OPS, 1026);
    assert_eq!(MAX_PAGE_OPS_V3, 4099);
    let mut fx = Fx::sized(10, 1024);
    let target = fx.k(1);
    fx.facts = (1..=1024).map(|i| efact(fx.k(i), true, None)).collect();
    fx.rows = (1..=1024)
        .map(|i| {
            let mut r = row(fx.k(i), mask(true, true, true, true, true));
            r.pools = vec![pool(1, 1, 2)];
            r.effects = vec![effect(2, 3)];
            r.order = order_in(target, i % 2 == 0);
            r.stats = vec![istat("s", i as i32)];
            r.tags = vec!["t".to_string()];
            r
        })
        .collect();
    fx.timeline = (1..=1024).map(|i| ((i % 7) as i32, fx.k(i))).collect();
    let mut p = Pair::new();
    let page = p.page(&fx);
    assert_eq!(page.enters, 1024);
    assert_eq!(page.ops.len(), 1024 + 1026);
    assert!(page.ops.len() <= MAX_PAGE_OPS_V3);
    assert!(page.ops[..1024]
        .iter()
        .all(|op| matches!(op, DeltaOp::EntityEnter { .. })));
    assert_eq!(page.ops[1024], enc_full());
    let mut sorted = fx.timeline.clone();
    sorted.sort();
    let want: Vec<TimelineEntry> = sorted
        .iter()
        .map(|&(key, k)| TimelineEntry {
            key,
            entity: p.view.net_id(k).expect("visible"),
        })
        .collect();
    assert_eq!(
        page.ops[1025],
        DeltaOp::Timeline {
            entries: Some(want)
        }
    );
    let target_net = p.view.net_id(target).map(NetId::get);
    let mut last = 0;
    for op in &page.ops[1026..] {
        let DeltaOp::EntityFacts { entity, facts } = op else {
            panic!("not EntityFacts: {op:?}")
        };
        assert!(entity.get() > last, "ascending NetId");
        last = entity.get();
        let k = p.view.entity(*entity).expect("mapped");
        let r = fx.rows.iter().find(|r| r.key == k).expect("row");
        assert_eq!(*facts, expect_image(r, target_net));
    }
    assert_eq!(last, 1024);
    assert_eq!(p.step(&fx), vec![]);
}

// ---------------------------------------------------------------------------
// C-9 / C13: host facts.
// ---------------------------------------------------------------------------

fn host(paused: Option<PauseReason>, timer: Option<u32>, bits: u8) -> HostInput {
    HostInput {
        paused,
        rights: PauseRights {
            pause: true,
            unpause: false,
        },
        auto_pause: AutoPauseToggles::from_bits(bits).expect("valid bits"),
        turn_timer_ms: timer,
    }
}
fn hg(pause_reason: bool, auto_pause: bool, turn_timer: bool) -> HostGrants {
    HostGrants {
        pause_reason,
        auto_pause,
        turn_timer,
    }
}
fn one(op: &DeltaOp) -> Vec<u8> {
    page_bytes(std::slice::from_ref(op))
}

#[test]
fn c13_literals() {
    let party = PauseReason::Auto(AutoPauseTrigger::PartyMemberDied);
    let input = host(Some(party), Some(30000), 0x15);
    let toggles = AutoPauseToggles::from_bits(0x15).expect("bits");
    let rights = PauseRights {
        pause: true,
        unpause: false,
    };
    let none = HostGrants::default();
    assert_eq!(
        clock_op(&input, none),
        DeltaOp::Clock { paused: Some(None) }
    );
    assert_eq!(
        host_facts_op(&input, none),
        DeltaOp::HostFacts(HostFacts {
            rights,
            auto_pause: None,
            turn_timer_ms: None
        })
    );
    let all = hg(true, true, true);
    assert_eq!(
        clock_op(&input, all),
        DeltaOp::Clock {
            paused: Some(Some(party))
        }
    );
    assert_eq!(
        host_facts_op(&input, all),
        DeltaOp::HostFacts(HostFacts {
            rights,
            auto_pause: Some(toggles),
            turn_timer_ms: Some(Some(30000))
        })
    );
    let no_timer = host(Some(party), None, 0x15);
    assert_eq!(
        host_facts_op(&no_timer, all),
        DeltaOp::HostFacts(HostFacts {
            rights,
            auto_pause: Some(toggles),
            turn_timer_ms: Some(None)
        })
    );
    // "Withheld" is not "no timer".
    assert_ne!(
        host_facts_op(&no_timer, none),
        host_facts_op(&no_timer, all)
    );
    let running = host(None, Some(5), 0);
    for b in 0..8 {
        assert_eq!(
            clock_op(&running, hg(b & 1 != 0, b & 2 != 0, b & 4 != 0)),
            DeltaOp::Clock { paused: None }
        );
    }
    assert_eq!(HostGrants::default(), hg(false, false, false));
}

#[test]
fn c13_every_grant_combination() {
    let mut reasons: Vec<Option<PauseReason>> = vec![
        None,
        Some(PauseReason::Player),
        Some(PauseReason::Operator),
        Some(PauseReason::Restore),
    ];
    for t in 0..5u8 {
        let trigger = AutoPauseTrigger::from_u8(t).expect("trigger");
        reasons.push(Some(PauseReason::Auto(trigger)));
    }
    for b in 0..8u8 {
        let g = hg(b & 1 != 0, b & 2 != 0, b & 4 != 0);
        for &paused in &reasons {
            for timer in [None, Some(0), Some(30000), Some(u32::MAX)] {
                for bits in [0, 0x15, 0x1F] {
                    let input = host(paused, timer, bits);
                    let want_clock = paused.map(|r| if g.pause_reason { Some(r) } else { None });
                    assert_eq!(clock_op(&input, g), DeltaOp::Clock { paused: want_clock });
                    let want = HostFacts {
                        rights: input.rights,
                        auto_pause: if g.auto_pause {
                            Some(input.auto_pause)
                        } else {
                            None
                        },
                        turn_timer_ms: if g.turn_timer { Some(timer) } else { None },
                    };
                    assert_eq!(host_facts_op(&input, g), DeltaOp::HostFacts(want));
                }
            }
        }
    }
    // Rights are always the viewer's own, under every grant.
    for (pause, unpause) in [(false, false), (true, false), (false, true), (true, true)] {
        let mut input = host(None, None, 0);
        input.rights = PauseRights { pause, unpause };
        let DeltaOp::HostFacts(f) = host_facts_op(&input, HostGrants::default()) else {
            panic!("HostFacts")
        };
        assert_eq!(f.rights, input.rights);
    }
}

#[test]
fn c13_withheld_inputs_do_not_move_bytes() {
    let party = PauseReason::Auto(AutoPauseTrigger::PartyMemberDied);
    let base = host(Some(party), Some(30000), 0x15);
    let none = HostGrants::default();
    let other = host(Some(PauseReason::Player), None, 0x0A);
    assert_eq!(one(&clock_op(&base, none)), one(&clock_op(&other, none)));
    assert_eq!(
        one(&host_facts_op(&base, none)),
        one(&host_facts_op(&other, none))
    );
    // Each grant on moves its own field's bytes, and only its own.
    assert_ne!(
        one(&clock_op(&base, hg(true, false, false))),
        one(&clock_op(&other, hg(true, false, false)))
    );
    let toggles_only = hg(false, true, false);
    assert_ne!(
        one(&host_facts_op(&base, toggles_only)),
        one(&host_facts_op(&other, toggles_only))
    );
    let timer_only = hg(false, false, true);
    let mut t2 = base;
    t2.turn_timer_ms = None;
    assert_ne!(
        one(&host_facts_op(&base, timer_only)),
        one(&host_facts_op(&t2, timer_only))
    );
    let mut a2 = base;
    a2.auto_pause = AutoPauseToggles::from_bits(0).expect("bits");
    assert_eq!(
        one(&host_facts_op(&base, timer_only)),
        one(&host_facts_op(&a2, timer_only))
    );
    assert_eq!(
        one(&host_facts_op(&base, toggles_only)),
        one(&host_facts_op(&t2, toggles_only))
    );
}

// ---------------------------------------------------------------------------
// C14: interest failure and error text.
// ---------------------------------------------------------------------------

#[test]
fn c14_interest_failures_leave_both_views_intact() {
    let mut fx = Fx::new();
    let mut p = Pair::new();
    p.page(&fx);
    let dup = efact(fx.k(1), true, None);
    fx.facts.push(dup);
    assert_eq!(
        p.err(&fx),
        FactError::Interest(InterestError::DuplicateFact)
    );
    fx.facts.pop();
    assert_eq!(p.step(&fx), vec![]);

    let mut big = Fx::sized(10, 1030);
    big.facts = (1..=1025).map(|i| efact(big.k(i), true, None)).collect();
    let mut p = Pair::new();
    assert_eq!(
        p.err(&big),
        FactError::Interest(InterestError::TooManyFacts)
    );
    big.facts.pop();
    assert_eq!(p.page(&big).enters, 1024);

    let mut despawned = Fx::new();
    despawned.despawned = vec![despawned.k(1); 1025];
    let mut p = Pair::new();
    assert_eq!(
        p.err(&despawned),
        FactError::Interest(InterestError::TooManyFacts)
    );
    despawned.despawned.pop();
    p.page(&despawned);
}

#[test]
fn c14_error_text_and_source() {
    let all = [
        (FactError::WrongEpoch, "WrongEpoch at projection/facts"),
        (FactError::TooManyFacts, "TooManyFacts at projection/facts"),
        (
            FactError::DuplicateFact,
            "DuplicateFact at projection/facts",
        ),
        (
            FactError::Interest(InterestError::WrongEpoch),
            "Interest at projection/facts",
        ),
        (
            FactError::Interest(InterestError::TooManyFacts),
            "Interest at projection/facts",
        ),
        (
            FactError::Interest(InterestError::DuplicateFact),
            "Interest at projection/facts",
        ),
        (
            FactError::Interest(InterestError::IdExhausted),
            "Interest at projection/facts",
        ),
    ];
    for (e, text) in all {
        assert_eq!(e.to_string(), text);
        let dynerr: &dyn Error = &e;
        assert!(dynerr.source().is_none());
    }
    assert_ne!(
        FactError::Interest(InterestError::WrongEpoch),
        FactError::Interest(InterestError::IdExhausted)
    );
}
