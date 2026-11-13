//! T140c implementer suite: the protocol v3 per-viewer fact filter
//! (`facts_v3`), derived from `tasks/T140.md` Behaviour C-1..C-9 and the
//! tests C1..C15, through the public API only.
//!
//! Expectations are hand-written whole `PageV3` / `DeltaOp` values or literal
//! constructions, never produced by the code under test. Every rejection is
//! compared against pre-call clones of both views and has a passing control.

use std::error::Error;

use crpg_core::{GenerationalArena, Ulid};
use crpg_net::codec_v3::{decode_delta, encode_delta, frame_spans};
use crpg_net::facts_v3::{
    clock_op, host_facts_op, project_page, EncounterGrants, EncounterInput, FactContext, FactError,
    FactMask, FactRow, FactView, HostGrants, HostInput, OrderInput, PageV3, MAX_FACT_PAGE_OPS,
    MAX_PAGE_OPS_V3,
};
use crpg_net::interest::{
    EntityFact, EntityKey, FieldMask, InterestError, InterestSet, InterestView, TurnState,
    ViewCursor, ViewerContext, MAX_PAGE_OPS,
};
use crpg_net::protocol::MAX_VISIBLE_ENTITIES;
use crpg_net::protocol_transfer::HealthFact;
use crpg_net::protocol_v3::{
    AutoPauseToggles, AutoPauseTrigger, ClockFact, DeltaFrame, DeltaOp, EffectFact, EncounterFacts,
    EncounterMode, FactSet, HostFacts, NetId, OrderFact, PauseReason, PauseRights, PoolFact,
    SessionEpoch, StatFact, StatValue, TimelineEntry, MAX_CLOCK_ELAPSED, MAX_FACT_EFFECTS,
    MAX_FACT_NAME_BYTES, MAX_FACT_POOLS, MAX_FACT_STATS, MAX_FACT_TAGS,
};

// ---------------------------------------------------------------------------
// Fixtures.
// ---------------------------------------------------------------------------

fn u(n: u128) -> Ulid {
    Ulid::from_u128(n)
}

fn net(n: u64) -> NetId {
    NetId::new(n).expect("fixture net id is nonzero")
}

fn epoch() -> SessionEpoch {
    SessionEpoch([0; 16])
}

fn other_epoch() -> SessionEpoch {
    SessionEpoch([1; 16])
}

/// A pool of entity keys in area `U(10)`, ascending, from one arena.
struct Keys {
    keys: Vec<EntityKey>,
}

impl Keys {
    fn new(n: usize) -> Self {
        let mut arena = GenerationalArena::<()>::new();
        let keys: Vec<EntityKey> = (0..n)
            .map(|_| EntityKey {
                area: u(10),
                entity: arena.insert(()),
            })
            .collect();
        assert!(keys.windows(2).all(|w| w[0] < w[1]), "arena ids ascend");
        Self { keys }
    }

    /// `k(1)` is the first key.
    fn k(&self, n: usize) -> EntityKey {
        self.keys[n - 1]
    }

    fn in_area(&self, n: usize, area: u128) -> EntityKey {
        EntityKey {
            area: u(area),
            entity: self.keys[n - 1].entity,
        }
    }
}

#[derive(Clone, Copy)]
struct Enc {
    area: Ulid,
    mode: EncounterMode,
    pending: Option<EncounterMode>,
    clock: Option<ClockFact>,
}

/// Everything one `project_page` call reads, owned.
#[derive(Clone)]
struct Scene {
    epoch: SessionEpoch,
    area: Ulid,
    state: Vec<EntityFact>,
    despawned: Vec<EntityKey>,
    turn: Option<TurnState>,
    turn_granted: bool,
    rows: Vec<FactRow>,
    encounter: Option<Enc>,
    timeline: Vec<(i32, EntityKey)>,
    grants: EncounterGrants,
    resend: bool,
}

impl Scene {
    fn run(&self, view: &mut InterestView, facts: &mut FactView) -> Result<PageV3, FactError> {
        let state = ViewerContext {
            epoch: self.epoch,
            tick: 9,
            interest: InterestSet { area: self.area },
            facts: &self.state,
            despawned: &self.despawned,
            turn: self.turn,
            turn_granted: self.turn_granted,
            resend_turn: false,
        };
        let encounter = self.encounter.map(|e| EncounterInput {
            area: e.area,
            mode: e.mode,
            pending_mode: e.pending,
            clock: e.clock,
            timeline: &self.timeline,
        });
        let context = FactContext {
            rows: &self.rows,
            encounter,
            grants: self.grants,
            resend: self.resend,
        };
        project_page(view, facts, &state, &context)
    }
}

fn fresh() -> (InterestView, FactView) {
    (InterestView::new(epoch()), FactView::new(epoch()))
}

fn state_row(key: EntityKey, presence: bool, health: Option<HealthFact>) -> EntityFact {
    EntityFact {
        key,
        fields: FieldMask {
            presence,
            health: health.is_some(),
        },
        health,
        resend_health: false,
    }
}

fn row(key: EntityKey, fields: FactMask) -> FactRow {
    FactRow {
        key,
        fields,
        pools: Vec::new(),
        effects: Vec::new(),
        order: None,
        stats: Vec::new(),
        tags: Vec::new(),
    }
}

fn pool(id: u128, current: u32, max: u32) -> PoolFact {
    PoolFact {
        pool: u(id),
        current,
        max,
    }
}

fn effect(id: u128, expires_round: u64) -> EffectFact {
    EffectFact {
        effect: u(id),
        expires_round,
    }
}

fn stat(name: &str, value: StatValue) -> StatFact {
    StatFact {
        name: name.to_owned(),
        value,
    }
}

fn mask(pools: bool, effects: bool, order: bool, stats: bool, tags: bool) -> FactMask {
    FactMask {
        pools,
        effects,
        order,
        stats,
        tags,
    }
}

fn pools_stats_mask() -> FactMask {
    mask(true, false, false, true, false)
}

fn all_mask() -> FactMask {
    mask(true, true, true, true, true)
}

fn all_grants() -> EncounterGrants {
    EncounterGrants {
        mode: true,
        pending_mode: true,
        clock: true,
        timeline: true,
    }
}

fn clock(round_ticks: u32, elapsed: u64) -> ClockFact {
    ClockFact {
        round_ticks,
        elapsed,
    }
}

/// `k2`'s fixture row: pools `[(U(1), 2, 3)]`, stats `[("might", Int(4))]`.
fn k2_row(fx: &Keys) -> FactRow {
    let mut r = row(fx.k(2), pools_stats_mask());
    r.pools = vec![pool(1, 2, 3)];
    r.stats = vec![stat("might", StatValue::Int(4))];
    r
}

/// `k3`'s fixture row: no grants, every family filled.
fn k3_row(fx: &Keys) -> FactRow {
    let mut r = row(fx.k(3), FactMask::default());
    r.pools = vec![pool(5, 1, 1)];
    r.effects = vec![effect(6, 3)];
    r.order = Some(OrderInput {
        ability: u(7),
        target: fx.k(1),
        repeat: true,
    });
    r.stats = vec![stat("grit", StatValue::Fixed(65_536))];
    r.tags = vec!["a".to_owned(), "b".to_owned()];
    r
}

fn k2_image() -> FactSet {
    FactSet {
        pools: Some(vec![pool(1, 2, 3)]),
        stats: Some(vec![stat("might", StatValue::Int(4))]),
        ..FactSet::default()
    }
}

/// The contract fixture: `k1..k3` visible, health granted for `k1` only,
/// `RealTime`, clock `{120, 240}`, timeline `[(5,k3), (-1,k2), (-1,k1)]`, all
/// four encounter grants, `resend` false.
fn base(fx: &Keys) -> Scene {
    Scene {
        epoch: epoch(),
        area: u(10),
        state: vec![
            state_row(
                fx.k(1),
                true,
                Some(HealthFact {
                    health: 10,
                    max_health: 10,
                    dead: false,
                }),
            ),
            state_row(fx.k(2), true, None),
            state_row(fx.k(3), true, None),
        ],
        despawned: Vec::new(),
        turn: None,
        turn_granted: false,
        rows: vec![k2_row(fx), k3_row(fx)],
        encounter: Some(Enc {
            area: u(10),
            mode: EncounterMode::RealTime,
            pending: None,
            clock: Some(clock(120, 240)),
        }),
        timeline: vec![(5, fx.k(3)), (-1, fx.k(2)), (-1, fx.k(1))],
        grants: all_grants(),
        resend: false,
    }
}

fn base_encounter_image() -> EncounterFacts {
    EncounterFacts {
        mode: Some(EncounterMode::RealTime),
        pending_mode: Some(None),
        clock: Some(Some(clock(120, 240))),
    }
}

fn tl(key: i32, entity: u64) -> TimelineEntry {
    TimelineEntry {
        key,
        entity: net(entity),
    }
}

fn enter(n: u64) -> DeltaOp {
    DeltaOp::EntityEnter { entity: net(n) }
}

fn leave(n: u64) -> DeltaOp {
    DeltaOp::EntityLeave { entity: net(n) }
}

fn entity_facts(n: u64, facts: FactSet) -> DeltaOp {
    DeltaOp::EntityFacts {
        entity: net(n),
        facts,
    }
}

fn base_first_ops() -> Vec<DeltaOp> {
    vec![
        enter(1),
        enter(2),
        enter(3),
        DeltaOp::Health {
            entity: net(1),
            health: 10,
            max_health: 10,
            dead: false,
        },
        DeltaOp::Encounter(base_encounter_image()),
        DeltaOp::Timeline {
            entries: Some(vec![tl(-1, 1), tl(-1, 2), tl(5, 3)]),
        },
        entity_facts(2, k2_image()),
    ]
}

/// Only the three fact ops of a page, in order.
fn fact_ops(page: &PageV3) -> Vec<DeltaOp> {
    page.ops
        .iter()
        .filter(|op| {
            matches!(
                op,
                DeltaOp::Encounter(_) | DeltaOp::Timeline { .. } | DeltaOp::EntityFacts { .. }
            )
        })
        .cloned()
        .collect()
}

fn page_bytes(page: &PageV3) -> Vec<u8> {
    encode_delta(&DeltaFrame {
        lane: 0,
        server_tick: 100,
        event_seq: 1,
        ops: page.ops.clone(),
    })
    .expect("fixture page encodes as one frame")
}

/// Runs `scene` on a fresh pair and returns the page.
fn first_page(scene: &Scene) -> PageV3 {
    let (mut view, mut facts) = fresh();
    scene.run(&mut view, &mut facts).expect("fixture page")
}

/// Asserts `scene` fails with `want` on `pair`, leaving both views equal to
/// their pre-call clones.
fn assert_rejected(scene: &Scene, pair: &(InterestView, FactView), want: FactError, why: &str) {
    let (mut view, mut facts) = pair.clone();
    let result = scene.run(&mut view, &mut facts);
    assert_eq!(result, Err(want), "{why}");
    assert_eq!(view, pair.0, "{why}: interest view changed");
    assert_eq!(facts, pair.1, "{why}: fact view changed");
}

// ---------------------------------------------------------------------------
// C1 and C-1: checks are all-or-nothing, in order.
// ---------------------------------------------------------------------------

#[test]
fn c1_checks_are_all_or_nothing() {
    let fx = Keys::new(1_100);
    let scene = base(&fx);

    // A cold pair and a warm pair (one page already applied) prove that no
    // error path touches remembered state.
    let cold = fresh();
    let mut warm = fresh();
    scene.run(&mut warm.0, &mut warm.1).expect("warm up");
    let pairs = [cold, warm];

    // Control: the scene is valid on both pairs.
    for pair in &pairs {
        let (mut v, mut f) = pair.clone();
        scene.run(&mut v, &mut f).expect("control passes");
    }

    // A FactView for another epoch.
    let wrong_facts = (InterestView::new(epoch()), FactView::new(other_epoch()));
    assert_rejected(&scene, &wrong_facts, FactError::WrongEpoch, "facts epoch");
    // An InterestView for another epoch.
    let wrong_view = (InterestView::new(other_epoch()), FactView::new(epoch()));
    assert_rejected(&scene, &wrong_view, FactError::WrongEpoch, "view epoch");

    for pair in &pairs {
        // A state for another epoch.
        let mut s = scene.clone();
        s.epoch = other_epoch();
        assert_rejected(&s, pair, FactError::WrongEpoch, "state epoch");

        // 1,024 rows pass, 1,025 do not.
        let mut ok = scene.clone();
        ok.rows = (1..=1_024)
            .map(|n| row(fx.k(n), FactMask::default()))
            .collect();
        let (mut v, mut f) = pair.clone();
        ok.run(&mut v, &mut f).expect("1,024 rows pass");
        let mut s = scene.clone();
        s.rows = (1..=1_025)
            .map(|n| row(fx.k(n), FactMask::default()))
            .collect();
        assert_rejected(&s, pair, FactError::TooManyFacts, "1,025 rows");

        // 1,025 rows that also hold a duplicate are TooManyFacts (count first).
        let mut s2 = s.clone();
        s2.rows[1_024] = row(fx.k(1), FactMask::default());
        assert_rejected(&s2, pair, FactError::TooManyFacts, "1,025 rows with dup");

        // 1,024 timeline entries pass, 1,025 do not.
        let mut ok = scene.clone();
        ok.timeline = (1..=1_024).map(|n| (n as i32, fx.k(n))).collect();
        let (mut v, mut f) = pair.clone();
        ok.run(&mut v, &mut f).expect("1,024 timeline entries pass");
        let mut s = scene.clone();
        s.timeline = (1..=1_025).map(|n| (n as i32, fx.k(n))).collect();
        assert_rejected(&s, pair, FactError::TooManyFacts, "1,025 timeline entries");

        // A duplicate row key: adjacent and not adjacent.
        let mut s = scene.clone();
        s.rows = vec![k2_row(&fx), k2_row(&fx), k3_row(&fx)];
        assert_rejected(&s, pair, FactError::DuplicateFact, "adjacent row dup");
        let mut s = scene.clone();
        s.rows = vec![k2_row(&fx), k3_row(&fx), k2_row(&fx)];
        assert_rejected(&s, pair, FactError::DuplicateFact, "split row dup");

        // A duplicate timeline key: adjacent and not adjacent, with a
        // different sort key too (the key, not the pair, is what repeats).
        let mut s = scene.clone();
        s.timeline = vec![(5, fx.k(3)), (-1, fx.k(2)), (-1, fx.k(2))];
        assert_rejected(&s, pair, FactError::DuplicateFact, "adjacent timeline dup");
        let mut s = scene.clone();
        s.timeline = vec![(5, fx.k(3)), (-1, fx.k(2)), (7, fx.k(1)), (9, fx.k(3))];
        assert_rejected(&s, pair, FactError::DuplicateFact, "split timeline dup");

        // Precedence: epoch beats count, count beats duplicates, duplicates
        // beat the interest projection.
        let mut s = scene.clone();
        s.epoch = other_epoch();
        s.rows = (1..=1_025)
            .map(|n| row(fx.k(n), FactMask::default()))
            .collect();
        assert_rejected(&s, pair, FactError::WrongEpoch, "epoch before count");
        let mut s = scene.clone();
        s.rows = vec![k2_row(&fx), k2_row(&fx)];
        s.state = vec![
            state_row(fx.k(1), true, None),
            state_row(fx.k(1), true, None),
        ];
        assert_rejected(&s, pair, FactError::DuplicateFact, "dup before interest");
        let mut s = scene.clone();
        s.state = vec![
            state_row(fx.k(1), true, None),
            state_row(fx.k(1), true, None),
        ];
        assert_rejected(
            &s,
            pair,
            FactError::Interest(InterestError::DuplicateFact),
            "interest dup is Interest",
        );
    }
}

#[test]
fn c1_foreign_area_encounter_still_counts_its_timeline() {
    // Reading of C-1: the timeline bound and duplicate check read the input
    // whenever an encounter is supplied, before and regardless of C-2's area
    // test and the timeline grant.
    let fx = Keys::new(1_100);
    let pair = fresh();

    let mut s = base(&fx);
    s.encounter.as_mut().unwrap().area = u(11);
    s.grants.timeline = false;
    let (mut v, mut f) = pair.clone();
    s.run(&mut v, &mut f).expect("foreign control passes");

    let mut over = s.clone();
    over.timeline = (1..=1_025).map(|n| (n as i32, fx.k(n))).collect();
    assert_rejected(&over, &pair, FactError::TooManyFacts, "foreign 1,025");
    let mut dup = s.clone();
    dup.timeline = vec![(1, fx.k(1)), (2, fx.k(2)), (3, fx.k(1))];
    assert_rejected(&dup, &pair, FactError::DuplicateFact, "foreign dup");

    // No encounter at all: nothing to count.
    let mut none = s.clone();
    none.encounter = None;
    none.timeline = (1..=1_025).map(|n| (n as i32, fx.k(n))).collect();
    let (mut v, mut f) = pair.clone();
    none.run(&mut v, &mut f)
        .expect("no encounter, no timeline check");
}

// ---------------------------------------------------------------------------
// C2, C3, C4, C5: layout, memory, change, resend.
// ---------------------------------------------------------------------------

#[test]
fn c2_first_page_layout() {
    let fx = Keys::new(4);
    let (mut view, mut facts) = fresh();
    let page = base(&fx).run(&mut view, &mut facts).expect("first page");
    assert_eq!(
        page,
        PageV3 {
            ops: base_first_ops(),
            enters: 3,
            cursor: ViewCursor {
                tick: 9,
                revision: 1
            },
        }
    );
}

#[test]
fn c3_unchanged_page_is_empty() {
    let fx = Keys::new(4);
    let scene = base(&fx);
    let (mut view, mut facts) = fresh();
    scene.run(&mut view, &mut facts).expect("first page");
    for _ in 0..3 {
        let page = scene.run(&mut view, &mut facts).expect("unchanged page");
        assert_eq!(
            page,
            PageV3 {
                ops: vec![],
                enters: 0,
                cursor: ViewCursor {
                    tick: 9,
                    revision: 1
                },
            }
        );
    }
}

#[test]
fn c4_change_and_revocation() {
    let fx = Keys::new(4);
    let mut scene = base(&fx);
    let (mut view, mut facts) = fresh();
    scene.run(&mut view, &mut facts).expect("first page");

    // A pool value changes: the whole new image, nothing else.
    scene.rows[0].pools = vec![pool(1, 1, 3)];
    let page = scene.run(&mut view, &mut facts).expect("pool change");
    assert_eq!(
        page.ops,
        vec![entity_facts(
            2,
            FactSet {
                pools: Some(vec![pool(1, 1, 3)]),
                stats: Some(vec![stat("might", StatValue::Int(4))]),
                ..FactSet::default()
            }
        )]
    );
    assert_eq!(page.enters, 0);

    // The pools grant is revoked: the op says so, stats stay.
    scene.rows[0].fields.pools = false;
    let page = scene.run(&mut view, &mut facts).expect("pools revoked");
    assert_eq!(
        page.ops,
        vec![entity_facts(
            2,
            FactSet {
                stats: Some(vec![stat("might", StatValue::Int(4))]),
                ..FactSet::default()
            }
        )]
    );

    // Every grant revoked: an all-withheld image, the forget op.
    scene.rows[0].fields = FactMask::default();
    let page = scene.run(&mut view, &mut facts).expect("all revoked");
    assert_eq!(page.ops, vec![entity_facts(2, FactSet::default())]);

    // Then nothing, and a value change under a withheld grant moves nothing.
    scene.rows[0].pools = vec![pool(1, 0, 3)];
    let page = scene.run(&mut view, &mut facts).expect("quiet");
    assert_eq!(page.ops, vec![]);
}

#[test]
fn c4_each_family_change_is_detected_alone() {
    let fx = Keys::new(4);
    let mut scene = base(&fx);
    scene.rows = vec![{
        let mut r = row(fx.k(2), all_mask());
        r.pools = vec![pool(1, 2, 3)];
        r.effects = vec![effect(3, 4)];
        r.order = Some(OrderInput {
            ability: u(9),
            target: fx.k(1),
            repeat: false,
        });
        r.stats = vec![stat("might", StatValue::Int(4))];
        r.tags = vec!["a".to_owned()];
        r
    }];
    let mut want = FactSet {
        pools: Some(vec![pool(1, 2, 3)]),
        effects: Some(vec![effect(3, 4)]),
        order: Some(Some(OrderFact {
            ability: u(9),
            target: net(1),
            repeat: false,
        })),
        stats: Some(vec![stat("might", StatValue::Int(4))]),
        tags: Some(vec!["a".to_owned()]),
    };
    let (mut view, mut facts) = fresh();
    let page = scene.run(&mut view, &mut facts).expect("first page");
    assert_eq!(fact_ops(&page)[2], entity_facts(2, want.clone()));

    // Effects.
    scene.rows[0].effects = vec![effect(3, 5)];
    want.effects = Some(vec![effect(3, 5)]);
    let page = scene.run(&mut view, &mut facts).expect("effects");
    assert_eq!(page.ops, vec![entity_facts(2, want.clone())]);

    // Order: ability, then repeat, then removed (disclosed nothing).
    scene.rows[0].order = Some(OrderInput {
        ability: u(8),
        target: fx.k(1),
        repeat: false,
    });
    want.order = Some(Some(OrderFact {
        ability: u(8),
        target: net(1),
        repeat: false,
    }));
    let page = scene.run(&mut view, &mut facts).expect("order ability");
    assert_eq!(page.ops, vec![entity_facts(2, want.clone())]);
    scene.rows[0].order.as_mut().unwrap().repeat = true;
    want.order = Some(Some(OrderFact {
        ability: u(8),
        target: net(1),
        repeat: true,
    }));
    let page = scene.run(&mut view, &mut facts).expect("order repeat");
    assert_eq!(page.ops, vec![entity_facts(2, want.clone())]);
    scene.rows[0].order = None;
    want.order = Some(None);
    let page = scene.run(&mut view, &mut facts).expect("order cleared");
    assert_eq!(page.ops, vec![entity_facts(2, want.clone())]);

    // Stats: value, then kind (Int(4) vs Fixed(4) are different images).
    scene.rows[0].stats = vec![stat("might", StatValue::Fixed(4))];
    want.stats = Some(vec![stat("might", StatValue::Fixed(4))]);
    let page = scene.run(&mut view, &mut facts).expect("stat kind");
    assert_eq!(page.ops, vec![entity_facts(2, want.clone())]);

    // Tags.
    scene.rows[0].tags = vec!["a".to_owned(), "b".to_owned()];
    want.tags = Some(vec!["a".to_owned(), "b".to_owned()]);
    let page = scene.run(&mut view, &mut facts).expect("tags");
    assert_eq!(page.ops, vec![entity_facts(2, want.clone())]);

    // Control: no change, no op.
    let page = scene.run(&mut view, &mut facts).expect("quiet");
    assert_eq!(page.ops, vec![]);
}

#[test]
fn c4_encounter_and_timeline_change_are_detected_alone() {
    let fx = Keys::new(4);
    let mut scene = base(&fx);
    let (mut view, mut facts) = fresh();
    scene.run(&mut view, &mut facts).expect("first page");
    let mut want = base_encounter_image();
    let timeline = |entries: Vec<TimelineEntry>| DeltaOp::Timeline {
        entries: Some(entries),
    };

    scene.encounter.as_mut().unwrap().mode = EncounterMode::TurnBased;
    want.mode = Some(EncounterMode::TurnBased);
    let page = scene.run(&mut view, &mut facts).expect("mode");
    assert_eq!(page.ops, vec![DeltaOp::Encounter(want)]);

    scene.encounter.as_mut().unwrap().pending = Some(EncounterMode::RealTime);
    want.pending_mode = Some(Some(EncounterMode::RealTime));
    let page = scene.run(&mut view, &mut facts).expect("pending");
    assert_eq!(page.ops, vec![DeltaOp::Encounter(want)]);

    scene.encounter.as_mut().unwrap().clock = Some(clock(120, 241));
    want.clock = Some(Some(clock(120, 241)));
    let page = scene.run(&mut view, &mut facts).expect("clock elapsed");
    assert_eq!(page.ops, vec![DeltaOp::Encounter(want)]);

    scene.encounter.as_mut().unwrap().clock = None;
    want.clock = Some(None);
    let page = scene.run(&mut view, &mut facts).expect("clock removed");
    assert_eq!(page.ops, vec![DeltaOp::Encounter(want)]);

    // A new tie-key order changes the timeline image only.
    scene.timeline = vec![(5, fx.k(3)), (-1, fx.k(2)), (0, fx.k(1))];
    let page = scene.run(&mut view, &mut facts).expect("timeline");
    assert_eq!(
        page.ops,
        vec![timeline(vec![tl(-1, 2), tl(0, 1), tl(5, 3)])]
    );

    // A granted empty timeline is a disclosed empty list, not withheld.
    scene.timeline = Vec::new();
    let page = scene.run(&mut view, &mut facts).expect("timeline empty");
    assert_eq!(page.ops, vec![timeline(vec![])]);
    let page = scene.run(&mut view, &mut facts).expect("quiet");
    assert_eq!(page.ops, vec![]);
}

#[test]
fn c5_resend_re_emits_only_disclosed_images() {
    let fx = Keys::new(4);
    let mut scene = base(&fx);
    let (mut view, mut facts) = fresh();
    scene.run(&mut view, &mut facts).expect("first page");

    scene.resend = true;
    let page = scene.run(&mut view, &mut facts).expect("resend");
    assert_eq!(
        page,
        PageV3 {
            ops: vec![
                DeltaOp::Encounter(base_encounter_image()),
                DeltaOp::Timeline {
                    entries: Some(vec![tl(-1, 1), tl(-1, 2), tl(5, 3)]),
                },
                entity_facts(2, k2_image()),
            ],
            enters: 0,
            cursor: ViewCursor {
                tick: 9,
                revision: 1
            },
        },
        "k3's withheld image is never sent; k1 has no row"
    );

    // Resend is one page only: the next call without it is empty again.
    scene.resend = false;
    assert_eq!(scene.run(&mut view, &mut facts).unwrap().ops, vec![]);

    // With nothing granted, resend still sends nothing (nothing to resend).
    let mut denied = base(&fx);
    for r in &mut denied.rows {
        r.fields = FactMask::default();
    }
    denied.grants = EncounterGrants::default();
    denied.resend = true;
    let (mut v, mut f) = fresh();
    let page = denied.run(&mut v, &mut f).expect("denied resend");
    assert_eq!(
        page.ops,
        vec![
            enter(1),
            enter(2),
            enter(3),
            DeltaOp::Health {
                entity: net(1),
                health: 10,
                max_health: 10,
                dead: false,
            }
        ]
    );

    // A granted-but-empty timeline is `Some`, so resend repeats it; a
    // foreign-area encounter is withheld, so resend repeats nothing.
    let mut empty_tl = base(&fx);
    empty_tl.timeline = Vec::new();
    let (mut v, mut f) = fresh();
    empty_tl.run(&mut v, &mut f).expect("first");
    empty_tl.resend = true;
    let page = empty_tl.run(&mut v, &mut f).expect("resend");
    assert_eq!(
        page.ops,
        vec![
            DeltaOp::Encounter(base_encounter_image()),
            DeltaOp::Timeline {
                entries: Some(vec![])
            },
            entity_facts(2, k2_image()),
        ]
    );
    let mut foreign = base(&fx);
    foreign.encounter.as_mut().unwrap().area = u(11);
    let (mut v, mut f) = fresh();
    foreign.run(&mut v, &mut f).expect("first");
    foreign.resend = true;
    let page = foreign.run(&mut v, &mut f).expect("resend");
    assert_eq!(page.ops, vec![entity_facts(2, k2_image())]);
}

// ---------------------------------------------------------------------------
// C-6: emission order.
// ---------------------------------------------------------------------------

#[test]
fn c6_emission_order_view_ops_then_encounter_timeline_entity_facts() {
    let fx = Keys::new(5);
    let mut scene = base(&fx);
    scene.turn = Some(TurnState {
        area: u(10),
        active: Some(fx.k(2).entity),
        round: 1,
    });
    scene.turn_granted = true;
    let (mut view, mut facts) = fresh();
    let page = scene.run(&mut view, &mut facts).expect("first page");
    let mut want = base_first_ops();
    want.insert(
        4,
        DeltaOp::Turn {
            active: Some(net(2)),
            round: 1,
        },
    );
    assert_eq!(page.ops, want);
    assert_eq!(page.enters, 3);

    // Second page: k1 leaves, k4 enters (id 4), the turn moves, the
    // encounter, the timeline and two rows change. Rows are given in a
    // scrambled order; EntityFacts still go by ascending NetId.
    let mut next = scene.clone();
    next.state = vec![
        state_row(fx.k(2), true, None),
        state_row(fx.k(3), true, None),
        state_row(fx.k(4), true, None),
    ];
    next.turn.as_mut().unwrap().round = 2;
    next.encounter.as_mut().unwrap().mode = EncounterMode::TurnBased;
    next.timeline = vec![(0, fx.k(4)), (0, fx.k(2))];
    let mut k4 = row(fx.k(4), mask(false, false, false, false, true));
    k4.tags = vec!["z".to_owned()];
    let mut k2 = k2_row(&fx);
    k2.pools = vec![pool(1, 3, 3)];
    next.rows = vec![k4, k3_row(&fx), k2];
    let page = next.run(&mut view, &mut facts).expect("second page");
    assert_eq!(
        page,
        PageV3 {
            ops: vec![
                enter(4),
                DeltaOp::Turn {
                    active: Some(net(2)),
                    round: 2,
                },
                leave(1),
                DeltaOp::Encounter(EncounterFacts {
                    mode: Some(EncounterMode::TurnBased),
                    ..base_encounter_image()
                }),
                DeltaOp::Timeline {
                    entries: Some(vec![tl(0, 2), tl(0, 4)]),
                },
                entity_facts(
                    2,
                    FactSet {
                        pools: Some(vec![pool(1, 3, 3)]),
                        stats: Some(vec![stat("might", StatValue::Int(4))]),
                        ..FactSet::default()
                    }
                ),
                entity_facts(
                    4,
                    FactSet {
                        tags: Some(vec!["z".to_owned()]),
                        ..FactSet::default()
                    }
                ),
            ],
            enters: 1,
            cursor: ViewCursor {
                tick: 9,
                revision: 2
            },
        }
    );
}

#[test]
fn c6_page_with_only_fact_changes_keeps_the_view_cursor() {
    // `cursor` and `enters` are the view's: fact ops alone do not advance it.
    let fx = Keys::new(4);
    let mut scene = base(&fx);
    let (mut view, mut facts) = fresh();
    let first = scene.run(&mut view, &mut facts).unwrap();
    scene.rows[0].pools = vec![pool(1, 0, 3)];
    let page = scene.run(&mut view, &mut facts).unwrap();
    assert_eq!(page.ops.len(), 1);
    assert_eq!(page.enters, 0);
    assert_eq!(page.cursor, first.cursor);
    assert_eq!(view.cursor(), first.cursor);
}

// ---------------------------------------------------------------------------
// C-7 and C6, C7: no grant, no op; hidden inputs do not move bytes.
// ---------------------------------------------------------------------------

#[test]
fn c6_no_grant_viewer_gets_no_fact_op() {
    let fx = Keys::new(4);
    let mut scene = base(&fx);
    scene.grants = EncounterGrants::default();
    for r in &mut scene.rows {
        r.fields = FactMask::default();
    }
    // The same state through the bare interest view, converted.
    let mut bare = InterestView::new(epoch());
    let bare_ops: Vec<DeltaOp> = {
        let state = ViewerContext {
            epoch: epoch(),
            tick: 9,
            interest: InterestSet { area: u(10) },
            facts: &scene.state,
            despawned: &[],
            turn: None,
            turn_granted: false,
            resend_turn: false,
        };
        bare.project_delta(&state)
            .unwrap()
            .ops
            .into_iter()
            .map(DeltaOp::from)
            .collect()
    };

    let (mut view, mut facts) = fresh();
    let page = scene.run(&mut view, &mut facts).expect("first page");
    assert_eq!(page.ops, bare_ops);
    assert_eq!(page.ops.len(), 4);
    assert!(fact_ops(&page).is_empty());

    // Every value changes; still no fact op, and no view op either.
    scene.rows = vec![
        {
            let mut r = row(fx.k(2), FactMask::default());
            r.pools = vec![pool(1, 1, 1)];
            r.stats = vec![stat("x", StatValue::Int(1))];
            r.tags = vec!["t".to_owned()];
            r.effects = vec![effect(2, 2)];
            r
        },
        k3_row(&fx),
    ];
    scene.encounter = Some(Enc {
        area: u(10),
        mode: EncounterMode::TurnBased,
        pending: Some(EncounterMode::RealTime),
        clock: Some(clock(7, 3)),
    });
    scene.timeline = vec![(3, fx.k(1)), (2, fx.k(2))];
    for resend in [false, true] {
        scene.resend = resend;
        let page = scene.run(&mut view, &mut facts).expect("changed page");
        assert_eq!(page.ops, vec![], "resend {resend}");
    }
}

#[test]
fn c7_hidden_inputs_do_not_move_bytes() {
    let fx = Keys::new(4);
    let baseline = |mutate: &dyn Fn(&mut Scene)| {
        let mut s = base(&fx);
        mutate(&mut s);
        page_bytes(&first_page(&s))
    };
    let reference = baseline(&|_| {});

    // k3's families: no grant, so any value gives identical bytes.
    let changed = baseline(&|s| {
        let mut r = row(fx.k(3), FactMask::default());
        r.pools = vec![pool(99, 9, 9)];
        r.effects = vec![effect(98, 9)];
        r.order = Some(OrderInput {
            ability: u(97),
            target: fx.k(2),
            repeat: false,
        });
        r.stats = vec![stat("other", StatValue::Int(-1))];
        r.tags = vec!["other".to_owned()];
        s.rows[1] = r;
    });
    assert_eq!(changed, reference, "k3 families");

    // k2's ungranted families: effects, order and tags, with pools/stats on.
    let changed = baseline(&|s| {
        s.rows[0].effects = vec![effect(1, 1)];
        s.rows[0].order = Some(OrderInput {
            ability: u(1),
            target: fx.k(1),
            repeat: true,
        });
        s.rows[0].tags = vec!["hidden".to_owned()];
    });
    assert_eq!(changed, reference, "k2 ungranted families");

    // Pending mode and timeline without their grants.
    let denied_ref = {
        let mut s = base(&fx);
        s.grants.pending_mode = false;
        s.grants.timeline = false;
        page_bytes(&first_page(&s))
    };
    let mut s = base(&fx);
    s.grants.pending_mode = false;
    s.grants.timeline = false;
    s.encounter.as_mut().unwrap().pending = Some(EncounterMode::TurnBased);
    s.timeline = vec![(1, fx.k(1)), (0, fx.k(3))];
    assert_eq!(page_bytes(&first_page(&s)), denied_ref, "pending, timeline");
    // Mode and clock without theirs.
    let mut s = base(&fx);
    s.grants.mode = false;
    s.grants.clock = false;
    let one = page_bytes(&first_page(&s));
    s.encounter.as_mut().unwrap().mode = EncounterMode::TurnBased;
    s.encounter.as_mut().unwrap().clock = Some(clock(5, 5));
    assert_eq!(page_bytes(&first_page(&s)), one, "mode, clock");

    // An encounter in another area, whatever it holds.
    let mut a = base(&fx);
    a.encounter = Some(Enc {
        area: u(11),
        mode: EncounterMode::RealTime,
        pending: None,
        clock: Some(clock(1, 1)),
    });
    let mut b = a.clone();
    b.encounter = Some(Enc {
        area: u(12),
        mode: EncounterMode::TurnBased,
        pending: Some(EncounterMode::RealTime),
        clock: None,
    });
    b.timeline = vec![(0, fx.k(1))];
    assert_eq!(page_bytes(&first_page(&a)), page_bytes(&first_page(&b)));

    // Controls: with the grant set, the same changes do move the bytes.
    let mut s = base(&fx);
    s.encounter.as_mut().unwrap().pending = Some(EncounterMode::TurnBased);
    assert_ne!(page_bytes(&first_page(&s)), reference, "pending granted");
    let mut s = base(&fx);
    s.timeline = vec![(1, fx.k(1)), (0, fx.k(3))];
    assert_ne!(page_bytes(&first_page(&s)), reference, "timeline granted");
    let mut s = base(&fx);
    s.rows[0].pools = vec![pool(1, 0, 3)];
    assert_ne!(page_bytes(&first_page(&s)), reference, "pools granted");
    let mut s = base(&fx);
    s.rows[1].fields.tags = true;
    assert_ne!(page_bytes(&first_page(&s)), reference, "k3 tags granted");
}

// ---------------------------------------------------------------------------
// Every grant on and off.
// ---------------------------------------------------------------------------

/// `k2`'s row with every family filled and granted per `mask`.
fn full_row(fx: &Keys, mask_value: FactMask) -> FactRow {
    let mut r = row(fx.k(2), mask_value);
    r.pools = vec![pool(1, 2, 3), pool(2, 0, 0)];
    r.effects = vec![effect(3, 4), effect(4, 5)];
    r.order = Some(OrderInput {
        ability: u(9),
        target: fx.k(1),
        repeat: true,
    });
    r.stats = vec![
        stat("might", StatValue::Int(4)),
        stat("agility", StatValue::Fixed(-65_536)),
    ];
    r.tags = vec!["alpha".to_owned(), "beta".to_owned()];
    r
}

fn full_image() -> FactSet {
    FactSet {
        pools: Some(vec![pool(1, 2, 3), pool(2, 0, 0)]),
        effects: Some(vec![effect(3, 4), effect(4, 5)]),
        order: Some(Some(OrderFact {
            ability: u(9),
            target: net(1),
            repeat: true,
        })),
        stats: Some(vec![
            stat("might", StatValue::Int(4)),
            stat("agility", StatValue::Fixed(-65_536)),
        ]),
        tags: Some(vec!["alpha".to_owned(), "beta".to_owned()]),
    }
}

/// Keeps only the families whose flag in `keep` is set (pools, effects,
/// order, stats, tags).
fn keep_only(image: FactSet, keep: [bool; 5]) -> FactSet {
    FactSet {
        pools: image.pools.filter(|_| keep[0]),
        effects: image.effects.filter(|_| keep[1]),
        order: image.order.filter(|_| keep[2]),
        stats: image.stats.filter(|_| keep[3]),
        tags: image.tags.filter(|_| keep[4]),
    }
}

#[test]
fn each_row_grant_on_and_off() {
    let fx = Keys::new(4);
    for family in 0..5 {
        for on in [true, false] {
            // `on`: only this family granted; `!on`: every family but it.
            let flags: [bool; 5] = core::array::from_fn(|i| (i == family) == on);
            let m = mask(flags[0], flags[1], flags[2], flags[3], flags[4]);
            let mut scene = base(&fx);
            scene.grants = EncounterGrants::default();
            scene.rows = vec![full_row(&fx, m)];
            let page = first_page(&scene);
            let image = keep_only(full_image(), flags);
            assert_eq!(
                fact_ops(&page),
                vec![entity_facts(2, image)],
                "family {family} on={on}"
            );
        }
    }
    // All off: no op at all (an all-withheld image equals the default).
    let mut scene = base(&fx);
    scene.grants = EncounterGrants::default();
    scene.rows = vec![full_row(&fx, FactMask::default())];
    assert_eq!(fact_ops(&first_page(&scene)), vec![]);
    // All on.
    scene.rows = vec![full_row(&fx, all_mask())];
    assert_eq!(
        fact_ops(&first_page(&scene)),
        vec![entity_facts(2, full_image())]
    );
}

#[test]
fn each_encounter_grant_on_and_off() {
    let fx = Keys::new(4);
    let image_for = |g: [bool; 4]| EncounterFacts {
        mode: g[0].then_some(EncounterMode::RealTime),
        pending_mode: g[1].then_some(None),
        clock: g[2].then_some(Some(clock(120, 240))),
    };
    for which in 0..4 {
        for on in [true, false] {
            let flags: [bool; 4] = core::array::from_fn(|i| (i == which) == on);
            let mut scene = base(&fx);
            scene.rows = Vec::new();
            scene.grants = EncounterGrants {
                mode: flags[0],
                pending_mode: flags[1],
                clock: flags[2],
                timeline: flags[3],
            };
            let page = first_page(&scene);
            let mut want = Vec::new();
            let image = image_for(flags);
            if !image.is_withheld() {
                want.push(DeltaOp::Encounter(image));
            }
            if flags[3] {
                want.push(DeltaOp::Timeline {
                    entries: Some(vec![tl(-1, 1), tl(-1, 2), tl(5, 3)]),
                });
            }
            assert_eq!(fact_ops(&page), want, "grant {which} on={on}");
        }
    }
}

#[test]
fn encounter_pending_mode_disclosed_values() {
    let fx = Keys::new(4);
    let mut scene = base(&fx);
    scene.rows = Vec::new();
    scene.grants = EncounterGrants {
        pending_mode: true,
        ..EncounterGrants::default()
    };
    // Granted and none: `Some(None)`, which is not the default image.
    let page = first_page(&scene);
    assert_eq!(
        fact_ops(&page),
        vec![DeltaOp::Encounter(EncounterFacts {
            pending_mode: Some(None),
            ..EncounterFacts::default()
        })]
    );
    scene.encounter.as_mut().unwrap().pending = Some(EncounterMode::TurnBased);
    assert_eq!(
        fact_ops(&first_page(&scene)),
        vec![DeltaOp::Encounter(EncounterFacts {
            pending_mode: Some(Some(EncounterMode::TurnBased)),
            ..EncounterFacts::default()
        })]
    );
}

// ---------------------------------------------------------------------------
// C8: orders.
// ---------------------------------------------------------------------------

#[test]
fn c8_order_rules() {
    let fx = Keys::new(5);
    let order_image = |scene: &Scene| -> Option<Option<OrderFact>> {
        let page = first_page(scene);
        let ops = fact_ops(&page);
        let found = ops.iter().find_map(|op| match op {
            DeltaOp::EntityFacts { entity, facts } if *entity == net(2) => Some(facts.order),
            _ => None,
        });
        found.unwrap_or(None)
    };
    let with_order = |order: Option<OrderInput>, fields: FactMask| {
        let mut scene = base(&fx);
        scene.grants = EncounterGrants::default();
        let mut r = row(fx.k(2), fields);
        r.pools = vec![pool(1, 2, 3)];
        r.order = order;
        scene.rows = vec![r];
        scene
    };
    let order_mask = mask(true, false, true, false, false);
    let input = |target: EntityKey, repeat: bool| OrderInput {
        ability: u(9),
        target,
        repeat,
    };

    // No order: disclosed nothing.
    assert_eq!(order_image(&with_order(None, order_mask)), Some(None));
    // Visible target (k1 = NetId 1).
    for repeat in [true, false] {
        assert_eq!(
            order_image(&with_order(Some(input(fx.k(1), repeat)), order_mask)),
            Some(Some(OrderFact {
                ability: u(9),
                target: net(1),
                repeat
            }))
        );
    }
    // Target k4 is in no state row: not nameable, the whole family withheld.
    assert_eq!(
        order_image(&with_order(Some(input(fx.k(4), true)), order_mask)),
        None
    );
    // Target k4 is in the state but with presence false: still hidden.
    let mut hidden = with_order(Some(input(fx.k(4), true)), order_mask);
    hidden.state.push(state_row(fx.k(4), false, None));
    assert_eq!(order_image(&hidden), None);
    // Target in another area is hidden too.
    assert_eq!(
        order_image(&with_order(
            Some(input(fx.in_area(1, 11), true)),
            order_mask
        )),
        None
    );
    // Control: the same hidden target becomes nameable once it is visible.
    let mut visible = with_order(Some(input(fx.k(4), true)), order_mask);
    visible.state.push(state_row(fx.k(4), true, None));
    assert_eq!(
        order_image(&visible),
        Some(Some(OrderFact {
            ability: u(9),
            target: net(4),
            repeat: true
        }))
    );
    // The default mask never discloses an order, present or absent.
    let plain = mask(true, false, false, false, false);
    assert_eq!(
        order_image(&with_order(Some(input(fx.k(1), true)), plain)),
        None
    );
    assert_eq!(order_image(&with_order(None, plain)), None);
    assert!(!FactMask::default().order);
}

#[test]
fn c8_order_target_changes_with_the_targets_id() {
    let fx = Keys::new(5);
    let mut scene = base(&fx);
    scene.grants = EncounterGrants::default();
    let mut r = row(fx.k(2), mask(false, false, true, false, false));
    r.order = Some(OrderInput {
        ability: u(9),
        target: fx.k(1),
        repeat: false,
    });
    scene.rows = vec![r];
    let (mut view, mut facts) = fresh();
    let page = scene.run(&mut view, &mut facts).unwrap();
    let want = |target: u64| {
        entity_facts(
            2,
            FactSet {
                order: Some(Some(OrderFact {
                    ability: u(9),
                    target: net(target),
                    repeat: false,
                })),
                ..FactSet::default()
            },
        )
    };
    assert_eq!(fact_ops(&page), vec![want(1)]);

    // The target leaves: the order can no longer name it, so the family is
    // withheld and the image changes (the old op's `NetId` is stale).
    let mut gone = scene.clone();
    gone.state.remove(0);
    let page = gone.run(&mut view, &mut facts).unwrap();
    assert_eq!(
        page.ops,
        vec![leave(1), entity_facts(2, FactSet::default())]
    );

    // It returns under a new id: the image names the new id.
    let page = scene.run(&mut view, &mut facts).unwrap();
    assert_eq!(
        page.ops,
        vec![
            enter(4),
            DeltaOp::Health {
                entity: net(4),
                health: 10,
                max_health: 10,
                dead: false,
            },
            want(4),
        ]
    );
}

// ---------------------------------------------------------------------------
// C9: over-limit families are withheld.
// ---------------------------------------------------------------------------

type Break = (&'static str, fn(&mut FactRow), fn(&mut FactSet));

#[test]
fn c9_overlimit_withheld() {
    let fx = Keys::new(4);
    let pools_n = |n: u128| (1..=n).map(|i| pool(i, 0, 1)).collect::<Vec<_>>();
    let effects_n = |n: u128| (1..=n).map(|i| effect(i, 1)).collect::<Vec<_>>();
    let stats_n = |n: usize| {
        (0..n)
            .map(|i| stat(&format!("s{i:02}"), StatValue::Int(i as i32)))
            .collect::<Vec<_>>()
    };
    let tags_n = |n: usize| (0..n).map(|i| format!("t{i}")).collect::<Vec<_>>();

    // Controls: every family exactly at its bound is disclosed in full, and
    // names of exactly 32 bytes pass.
    let mut at_bound = full_row(&fx, all_mask());
    at_bound.pools = pools_n(MAX_FACT_POOLS as u128);
    at_bound.effects = effects_n(MAX_FACT_EFFECTS as u128);
    let mut stats = stats_n(MAX_FACT_STATS - 1);
    stats.push(stat(&"n".repeat(MAX_FACT_NAME_BYTES), StatValue::Fixed(1)));
    at_bound.stats = stats.clone();
    let mut tags = tags_n(MAX_FACT_TAGS - 1);
    tags.push("u".repeat(MAX_FACT_NAME_BYTES));
    at_bound.tags = tags.clone();
    let mut scene = base(&fx);
    scene.grants = EncounterGrants::default();
    scene.rows = vec![at_bound.clone()];
    let page = first_page(&scene);
    assert_eq!(
        fact_ops(&page),
        vec![entity_facts(
            2,
            FactSet {
                pools: Some(pools_n(MAX_FACT_POOLS as u128)),
                effects: Some(effects_n(MAX_FACT_EFFECTS as u128)),
                order: full_image().order,
                stats: Some(stats),
                tags: Some(tags),
            }
        )]
    );

    // One past each bound, and each shape fault, withholds only its family
    // and raises no error.
    let breaks: Vec<Break> = vec![
        (
            "17 pools",
            |r| r.pools = (1..=17).map(|i| pool(i, 0, 1)).collect(),
            |s| s.pools = None,
        ),
        (
            "current > max",
            |r| r.pools = vec![pool(1, 2, 1)],
            |s| s.pools = None,
        ),
        (
            "duplicate pool, split",
            |r| r.pools = vec![pool(1, 0, 1), pool(2, 0, 1), pool(1, 0, 1)],
            |s| s.pools = None,
        ),
        (
            "17 effects",
            |r| r.effects = (1..=17).map(|i| effect(i, 1)).collect(),
            |s| s.effects = None,
        ),
        (
            "effects descending",
            |r| r.effects = vec![effect(4, 1), effect(3, 1)],
            |s| s.effects = None,
        ),
        (
            "effect id repeated",
            |r| r.effects = vec![effect(3, 1), effect(3, 2)],
            |s| s.effects = None,
        ),
        (
            "33 stats",
            |r| {
                r.stats = (0..33)
                    .map(|i| stat(&format!("s{i:02}"), StatValue::Int(0)))
                    .collect()
            },
            |s| s.stats = None,
        ),
        (
            "33-byte stat name",
            |r| r.stats = vec![stat(&"n".repeat(33), StatValue::Int(0))],
            |s| s.stats = None,
        ),
        (
            "empty stat name",
            |r| r.stats = vec![stat("", StatValue::Int(0))],
            |s| s.stats = None,
        ),
        (
            "stat name repeated, split",
            |r| {
                r.stats = vec![
                    stat("a", StatValue::Int(0)),
                    stat("b", StatValue::Int(0)),
                    stat("a", StatValue::Fixed(0)),
                ]
            },
            |s| s.stats = None,
        ),
        (
            "9 tags",
            |r| r.tags = (0..9).map(|i| format!("t{i}")).collect(),
            |s| s.tags = None,
        ),
        (
            "33-byte tag",
            |r| r.tags = vec!["t".repeat(33)],
            |s| s.tags = None,
        ),
        (
            "empty tag",
            |r| r.tags = vec![String::new()],
            |s| s.tags = None,
        ),
        (
            "tags unsorted",
            |r| r.tags = vec!["b".to_owned(), "a".to_owned()],
            |s| s.tags = None,
        ),
        (
            "tag repeated",
            |r| r.tags = vec!["a".to_owned(), "a".to_owned()],
            |s| s.tags = None,
        ),
    ];
    for (name, break_row, clear) in breaks {
        let mut broken = full_row(&fx, all_mask());
        break_row(&mut broken);
        let mut scene = base(&fx);
        scene.grants = EncounterGrants::default();
        scene.rows = vec![broken];
        let page = first_page(&scene);
        let mut want = full_image();
        clear(&mut want);
        assert_eq!(fact_ops(&page), vec![entity_facts(2, want)], "{name}");
    }
}

// ---------------------------------------------------------------------------
// C10: a key that re-enters has a new id.
// ---------------------------------------------------------------------------

#[test]
fn c10_reentry_new_id() {
    let fx = Keys::new(5);
    let scene = base(&fx);
    let (mut view, mut facts) = fresh();
    scene.run(&mut view, &mut facts).expect("first");

    let mut away = scene.clone();
    away.state[1].fields.presence = false;
    let page = away.run(&mut view, &mut facts).expect("k2 leaves");
    // The timeline loses its k2 entry (the entity is hidden); no EntityFacts.
    assert_eq!(
        page.ops,
        vec![
            leave(2),
            DeltaOp::Timeline {
                entries: Some(vec![tl(-1, 1), tl(5, 3)]),
            },
        ]
    );

    // The same image under a new id is a new subject: the op is sent.
    let page = scene.run(&mut view, &mut facts).expect("k2 returns");
    assert_eq!(
        page.ops,
        vec![
            enter(4),
            DeltaOp::Timeline {
                entries: Some(vec![tl(-1, 1), tl(-1, 4), tl(5, 3)]),
            },
            entity_facts(4, k2_image()),
        ]
    );
    assert_eq!(page.enters, 1);

    // And then nothing.
    assert_eq!(scene.run(&mut view, &mut facts).unwrap().ops, vec![]);
}

#[test]
fn c10_a_leaving_key_forgets_its_image_even_while_rowless() {
    let fx = Keys::new(5);
    let mut scene = base(&fx);
    scene.grants = EncounterGrants::default();
    let (mut view, mut facts) = fresh();
    scene.run(&mut view, &mut facts).expect("first");

    // k2 leaves with its row gone; it returns with no row: the new id has
    // the default image, so nothing is sent for it.
    let mut away = scene.clone();
    away.state[1].fields.presence = false;
    away.rows = vec![k3_row(&fx)];
    assert_eq!(away.run(&mut view, &mut facts).unwrap().ops, vec![leave(2)]);
    let mut back = scene.clone();
    back.rows = vec![k3_row(&fx)];
    assert_eq!(back.run(&mut view, &mut facts).unwrap().ops, vec![enter(4)]);
}

#[test]
fn timeline_keeps_authority_order_not_net_id_order() {
    let fx = Keys::new(5);
    let mut scene = base(&fx);
    scene.rows = Vec::new();
    scene.grants = EncounterGrants {
        timeline: true,
        ..EncounterGrants::default()
    };
    // k2 re-enters as NetId 4, so ties no longer run in NetId order.
    scene.timeline = vec![(0, fx.k(3)), (0, fx.k(2)), (0, fx.k(1))];
    let (mut view, mut facts) = fresh();
    scene.run(&mut view, &mut facts).unwrap();
    let mut away = scene.clone();
    away.state[1].fields.presence = false;
    away.run(&mut view, &mut facts).unwrap();
    let page = scene.run(&mut view, &mut facts).unwrap();
    assert_eq!(
        page.ops,
        vec![
            enter(4),
            DeltaOp::Timeline {
                entries: Some(vec![tl(0, 1), tl(0, 4), tl(0, 3)]),
            },
        ]
    );
}

#[test]
fn timeline_drops_hidden_and_foreign_entities() {
    let fx = Keys::new(5);
    let mut scene = base(&fx);
    scene.rows = Vec::new();
    scene.timeline = vec![
        (4, fx.k(5)),           // never in the state
        (3, fx.in_area(1, 11)), // same entity id, foreign area
        (2, fx.k(3)),
        (1, fx.k(1)),
    ];
    scene.state.push(state_row(fx.k(4), false, None)); // present = false
    scene.timeline.push((0, fx.k(4)));
    let page = first_page(&scene);
    assert_eq!(
        fact_ops(&page),
        vec![
            DeltaOp::Encounter(base_encounter_image()),
            DeltaOp::Timeline {
                entries: Some(vec![tl(1, 1), tl(2, 3)]),
            },
        ]
    );
}

// ---------------------------------------------------------------------------
// C11: an encounter in another area.
// ---------------------------------------------------------------------------

#[test]
fn c11_foreign_area_encounter() {
    let fx = Keys::new(4);
    let mut scene = base(&fx);
    let (mut view, mut facts) = fresh();
    scene.run(&mut view, &mut facts).expect("first");

    scene.encounter.as_mut().unwrap().area = u(11);
    let page = scene.run(&mut view, &mut facts).expect("foreign");
    assert_eq!(
        page.ops,
        vec![
            DeltaOp::Encounter(EncounterFacts::default()),
            DeltaOp::Timeline { entries: None },
        ]
    );
    // The forget op is sent once.
    assert_eq!(scene.run(&mut view, &mut facts).unwrap().ops, vec![]);

    // Moving back discloses it again; the encounter being absent forgets it
    // the same way.
    scene.encounter.as_mut().unwrap().area = u(10);
    let page = scene.run(&mut view, &mut facts).expect("back");
    assert_eq!(
        page.ops,
        vec![
            DeltaOp::Encounter(base_encounter_image()),
            DeltaOp::Timeline {
                entries: Some(vec![tl(-1, 1), tl(-1, 2), tl(5, 3)]),
            },
        ]
    );
    scene.encounter = None;
    let page = scene.run(&mut view, &mut facts).expect("absent");
    assert_eq!(
        page.ops,
        vec![
            DeltaOp::Encounter(EncounterFacts::default()),
            DeltaOp::Timeline { entries: None },
        ]
    );
    assert_eq!(scene.run(&mut view, &mut facts).unwrap().ops, vec![]);

    // A foreign encounter on a fresh view says nothing at all (nothing to
    // forget); and the grant gates the timeline forget op too.
    let mut foreign = base(&fx);
    foreign.encounter.as_mut().unwrap().area = u(11);
    let page = first_page(&foreign);
    assert!(fact_ops(&page)
        .iter()
        .all(|op| matches!(op, DeltaOp::EntityFacts { .. })));
}

// ---------------------------------------------------------------------------
// C12: the page bound.
// ---------------------------------------------------------------------------

#[test]
fn c12_page_bound() {
    assert_eq!(MAX_FACT_PAGE_OPS, 1_026);
    assert_eq!(MAX_PAGE_OPS_V3, 4_099);
    assert_eq!(MAX_PAGE_OPS_V3, MAX_PAGE_OPS + MAX_FACT_PAGE_OPS);
    assert_eq!(MAX_FACT_PAGE_OPS, MAX_VISIBLE_ENTITIES + 2);

    let fx = Keys::new(MAX_VISIBLE_ENTITIES);
    let mut scene = base(&fx);
    scene.state = (1..=MAX_VISIBLE_ENTITIES)
        .map(|n| state_row(fx.k(n), true, None))
        .collect();
    scene.rows = (1..=MAX_VISIBLE_ENTITIES)
        .map(|n| {
            let mut r = row(fx.k(n), all_mask());
            r.pools = (1..=16).map(|i| pool(i, 1, 2)).collect();
            r.effects = (1..=16).map(|i| effect(i, 9)).collect();
            r.order = Some(OrderInput {
                ability: u(9),
                target: fx.k(1),
                repeat: true,
            });
            r.stats = (0..32)
                .map(|i| {
                    stat(
                        &format!("stat-{i:02}-{}", "x".repeat(20)),
                        StatValue::Int(i),
                    )
                })
                .collect();
            r.tags = (0..8)
                .map(|i| format!("tag-{i}-{}", "y".repeat(20)))
                .collect();
            r
        })
        .collect();
    scene.timeline = (1..=MAX_VISIBLE_ENTITIES)
        .map(|n| (n as i32, fx.k(n)))
        .collect();
    let (mut view, mut facts) = fresh();
    let page = scene.run(&mut view, &mut facts).expect("full page");

    assert_eq!(page.enters, MAX_VISIBLE_ENTITIES);
    assert_eq!(fact_ops(&page).len(), MAX_FACT_PAGE_OPS);
    assert_eq!(page.ops.len(), MAX_VISIBLE_ENTITIES + MAX_FACT_PAGE_OPS);
    assert!(page.ops.len() <= MAX_PAGE_OPS_V3);
    // Layout: enters, Encounter, Timeline, then EntityFacts by NetId.
    assert!(page.ops[..MAX_VISIBLE_ENTITIES]
        .iter()
        .enumerate()
        .all(|(i, op)| *op == enter(i as u64 + 1)));
    assert!(matches!(
        page.ops[MAX_VISIBLE_ENTITIES],
        DeltaOp::Encounter(_)
    ));
    assert!(matches!(
        &page.ops[MAX_VISIBLE_ENTITIES + 1],
        DeltaOp::Timeline { entries: Some(e) } if e.len() == MAX_VISIBLE_ENTITIES
    ));
    for (i, op) in page.ops[MAX_VISIBLE_ENTITIES + 2..].iter().enumerate() {
        match op {
            DeltaOp::EntityFacts { entity, facts } => {
                assert_eq!(*entity, net(i as u64 + 1));
                assert_eq!(facts.pools.as_ref().map(Vec::len), Some(16));
                assert_eq!(facts.stats.as_ref().map(Vec::len), Some(32));
            }
            other => panic!("unexpected op {other:?}"),
        }
    }

    // A page is not a frame: the one splitter cuts it, and every span encodes
    // and decodes back to exactly its ops.
    let spans = frame_spans(&page.ops).expect("spans");
    assert!(spans.len() > 1);
    let mut rebuilt = Vec::new();
    for span in spans {
        let frame = DeltaFrame {
            lane: 0,
            server_tick: u64::MAX,
            event_seq: u64::MAX,
            ops: page.ops[span].to_vec(),
        };
        let bytes = encode_delta(&frame).expect("span encodes");
        rebuilt.extend(decode_delta(&bytes).expect("span decodes").ops);
    }
    assert_eq!(rebuilt, page.ops);
}

// ---------------------------------------------------------------------------
// C13: host facts by grant.
// ---------------------------------------------------------------------------

fn host_input(paused: Option<PauseReason>, timer: Option<u32>) -> HostInput {
    HostInput {
        paused,
        rights: PauseRights {
            pause: true,
            unpause: false,
        },
        auto_pause: AutoPauseToggles::from_bits(0x15).unwrap(),
        turn_timer_ms: timer,
    }
}

fn grants_from(bits: u8) -> HostGrants {
    HostGrants {
        pause_reason: bits & 1 != 0,
        auto_pause: bits & 2 != 0,
        turn_timer: bits & 4 != 0,
    }
}

fn op_bytes(op: DeltaOp) -> Vec<u8> {
    encode_delta(&DeltaFrame {
        lane: 0,
        server_tick: 100,
        event_seq: 1,
        ops: vec![op],
    })
    .unwrap()
}

#[test]
fn c13_host_fact_grants() {
    let reason = PauseReason::Auto(AutoPauseTrigger::PartyMemberDied);
    let input = host_input(Some(reason), Some(30_000));
    let rights = PauseRights {
        pause: true,
        unpause: false,
    };
    let toggles = AutoPauseToggles::from_bits(0x15).unwrap();

    // Defaults deny the three grantable facts and nothing else.
    assert_eq!(
        clock_op(&input, HostGrants::default()),
        DeltaOp::Clock { paused: Some(None) }
    );
    assert_eq!(
        host_facts_op(&input, HostGrants::default()),
        DeltaOp::HostFacts(HostFacts {
            rights,
            auto_pause: None,
            turn_timer_ms: None,
        })
    );
    // All three grants.
    let all = grants_from(7);
    assert_eq!(
        clock_op(&input, all),
        DeltaOp::Clock {
            paused: Some(Some(reason))
        }
    );
    assert_eq!(
        host_facts_op(&input, all),
        DeltaOp::HostFacts(HostFacts {
            rights,
            auto_pause: Some(toggles),
            turn_timer_ms: Some(Some(30_000)),
        })
    );
    // No timer running is disclosed as `Some(None)`, which is not withheld.
    assert_eq!(
        host_facts_op(&host_input(Some(reason), None), all),
        DeltaOp::HostFacts(HostFacts {
            rights,
            auto_pause: Some(toggles),
            turn_timer_ms: Some(None),
        })
    );
    // Running is `Clock { None }` under any grants.
    for bits in 0..8u8 {
        assert_eq!(
            clock_op(&host_input(None, Some(1)), grants_from(bits)),
            DeltaOp::Clock { paused: None },
            "running, grants {bits:03b}"
        );
    }
}

#[test]
fn c13_every_grant_combination() {
    let reasons = [
        PauseReason::Player,
        PauseReason::Auto(AutoPauseTrigger::EncounterStarted),
        PauseReason::Auto(AutoPauseTrigger::PartyMemberDied),
        PauseReason::Operator,
        PauseReason::Restore,
    ];
    let rights_all = [
        PauseRights {
            pause: false,
            unpause: false,
        },
        PauseRights {
            pause: true,
            unpause: false,
        },
        PauseRights {
            pause: false,
            unpause: true,
        },
        PauseRights {
            pause: true,
            unpause: true,
        },
    ];
    for bits in 0..8u8 {
        let grants = grants_from(bits);
        for reason in reasons {
            for rights in rights_all {
                for timer in [None, Some(0), Some(30_000), Some(u32::MAX)] {
                    let input = HostInput {
                        paused: Some(reason),
                        rights,
                        auto_pause: AutoPauseToggles::from_bits(0x0A).unwrap(),
                        turn_timer_ms: timer,
                    };
                    assert_eq!(
                        clock_op(&input, grants),
                        DeltaOp::Clock {
                            paused: Some(grants.pause_reason.then_some(reason))
                        }
                    );
                    assert_eq!(
                        host_facts_op(&input, grants),
                        DeltaOp::HostFacts(HostFacts {
                            rights,
                            auto_pause: grants
                                .auto_pause
                                .then_some(AutoPauseToggles::from_bits(0x0A).unwrap()),
                            turn_timer_ms: grants.turn_timer.then_some(timer),
                        }),
                        "grants {bits:03b}"
                    );
                }
            }
        }
    }
}

#[test]
fn c13_withheld_host_fields_do_not_move_bytes() {
    let base_input = host_input(Some(PauseReason::Player), Some(5_000));
    let changed = HostInput {
        paused: Some(PauseReason::Restore),
        auto_pause: AutoPauseToggles::from_bits(0x0A).unwrap(),
        turn_timer_ms: None,
        ..base_input
    };
    let deny = HostGrants::default();
    assert_eq!(
        op_bytes(clock_op(&base_input, deny)),
        op_bytes(clock_op(&changed, deny))
    );
    assert_eq!(
        op_bytes(host_facts_op(&base_input, deny)),
        op_bytes(host_facts_op(&changed, deny))
    );
    // Each grant alone un-hides only its own field, and the bytes move.
    let reason_only = HostGrants {
        pause_reason: true,
        ..deny
    };
    assert_ne!(
        op_bytes(clock_op(&base_input, reason_only)),
        op_bytes(clock_op(&changed, reason_only))
    );
    let toggles_only = HostGrants {
        auto_pause: true,
        ..deny
    };
    let timer_only = HostGrants {
        turn_timer: true,
        ..deny
    };
    let toggles_differ = HostInput {
        turn_timer_ms: base_input.turn_timer_ms,
        ..changed
    };
    assert_eq!(
        op_bytes(host_facts_op(&base_input, timer_only)),
        op_bytes(host_facts_op(
            &HostInput {
                auto_pause: changed.auto_pause,
                ..base_input
            },
            timer_only
        )),
        "toggles are not sent under the timer grant alone"
    );
    assert_ne!(
        op_bytes(host_facts_op(&base_input, toggles_only)),
        op_bytes(host_facts_op(&toggles_differ, toggles_only))
    );
    assert_ne!(
        op_bytes(host_facts_op(&base_input, timer_only)),
        op_bytes(host_facts_op(&changed, timer_only))
    );
    // "Withheld" is not "no timer running".
    let running = HostInput {
        turn_timer_ms: None,
        ..base_input
    };
    assert_ne!(
        op_bytes(host_facts_op(&running, deny)),
        op_bytes(host_facts_op(&running, timer_only))
    );
}

// ---------------------------------------------------------------------------
// C14: the interest arm, and error text.
// ---------------------------------------------------------------------------

#[test]
fn c14_interest_failure_and_error_text() {
    let fx = Keys::new(1_100);
    let scene = base(&fx);
    let cold = fresh();
    let mut warm = fresh();
    scene.run(&mut warm.0, &mut warm.1).expect("warm up");

    for pair in [cold, warm] {
        // Two state rows for k1 (one non-adjacent).
        let mut s = scene.clone();
        s.state = vec![
            state_row(fx.k(1), true, None),
            state_row(fx.k(2), true, None),
            state_row(fx.k(1), true, None),
        ];
        assert_rejected(
            &s,
            &pair,
            FactError::Interest(InterestError::DuplicateFact),
            "state dup",
        );
        // 1,025 state rows.
        let mut s = scene.clone();
        s.state = (1..=1_025)
            .map(|n| state_row(fx.k(n), true, None))
            .collect();
        assert_rejected(
            &s,
            &pair,
            FactError::Interest(InterestError::TooManyFacts),
            "state 1,025",
        );
        // 1,025 despawned keys.
        let mut s = scene.clone();
        s.despawned = (1..=1_025).map(|n| fx.k(n)).collect();
        assert_rejected(
            &s,
            &pair,
            FactError::Interest(InterestError::TooManyFacts),
            "despawned 1,025",
        );
        // Control: 1,024 state rows and 1,024 despawned keys pass.
        let mut s = scene.clone();
        s.state = (1..=1_024)
            .map(|n| state_row(fx.k(n), true, None))
            .collect();
        s.despawned = (1..=1_024).map(|n| fx.k(n)).collect();
        let (mut v, mut f) = pair.clone();
        s.run(&mut v, &mut f).expect("1,024 pass");
    }

    let texts = [
        (FactError::WrongEpoch, "WrongEpoch at projection/facts"),
        (FactError::TooManyFacts, "TooManyFacts at projection/facts"),
        (
            FactError::DuplicateFact,
            "DuplicateFact at projection/facts",
        ),
    ];
    for (error, text) in texts {
        assert_eq!(error.to_string(), text);
        assert!(error.source().is_none());
    }
    for inner in [
        InterestError::WrongEpoch,
        InterestError::TooManyFacts,
        InterestError::DuplicateFact,
        InterestError::IdExhausted,
    ] {
        let error = FactError::Interest(inner);
        assert_eq!(error.to_string(), "Interest at projection/facts");
        assert!(error.source().is_none());
    }
}

// ---------------------------------------------------------------------------
// C15: a rowless visible key, and an invalid clock.
// ---------------------------------------------------------------------------

#[test]
fn c15_rowless_key() {
    let fx = Keys::new(4);
    let scene = base(&fx);
    let (mut view, mut facts) = fresh();
    scene.run(&mut view, &mut facts).expect("first");

    let mut rowless = scene.clone();
    rowless.rows = vec![k3_row(&fx)];
    let page = rowless.run(&mut view, &mut facts).expect("k2 rowless");
    assert_eq!(page.ops, vec![entity_facts(2, FactSet::default())]);
    assert_eq!(rowless.run(&mut view, &mut facts).unwrap().ops, vec![]);

    // k1 was visible and rowless from the start: it never had an op, in any
    // of the pages above.
    let (mut v, mut f) = fresh();
    let page = scene.run(&mut v, &mut f).unwrap();
    assert!(page.ops.iter().all(|op| !matches!(
        op,
        DeltaOp::EntityFacts { entity, .. } if *entity == net(1)
    )));

    // Control: the row coming back re-emits the image.
    let page = scene.run(&mut view, &mut facts).expect("row returns");
    assert_eq!(page.ops, vec![entity_facts(2, k2_image())]);
}

#[test]
fn c15_invalid_clock_is_withheld_never_refused() {
    let fx = Keys::new(4);
    let want = |clock_image: Option<Option<ClockFact>>| {
        let mut ops = base_first_ops();
        ops[4] = DeltaOp::Encounter(EncounterFacts {
            clock: clock_image,
            ..base_encounter_image()
        });
        ops
    };
    let page_for = |c: Option<ClockFact>| {
        let mut scene = base(&fx);
        scene.encounter.as_mut().unwrap().clock = c;
        first_page(&scene).ops
    };

    // Invalid: zero round ticks, and elapsed one past `i32::MAX`.
    assert_eq!(page_for(Some(clock(0, 240))), want(None));
    assert_eq!(page_for(Some(clock(120, 2_147_483_648))), want(None));
    assert_eq!(page_for(Some(clock(120, u64::MAX))), want(None));
    // Controls: the bounds themselves are disclosed; no clock is `Some(None)`.
    assert_eq!(MAX_CLOCK_ELAPSED, 2_147_483_647);
    assert_eq!(
        page_for(Some(clock(120, 2_147_483_647))),
        want(Some(Some(clock(120, 2_147_483_647))))
    );
    assert_eq!(page_for(Some(clock(1, 0))), want(Some(Some(clock(1, 0)))));
    assert_eq!(page_for(None), want(Some(None)));

    // An invalid clock that becomes valid is a change and is sent.
    let mut scene = base(&fx);
    scene.encounter.as_mut().unwrap().clock = Some(clock(0, 1));
    let (mut view, mut facts) = fresh();
    scene.run(&mut view, &mut facts).unwrap();
    assert_eq!(scene.run(&mut view, &mut facts).unwrap().ops, vec![]);
    scene.encounter.as_mut().unwrap().clock = Some(clock(1, 1));
    assert_eq!(
        scene.run(&mut view, &mut facts).unwrap().ops,
        vec![DeltaOp::Encounter(EncounterFacts {
            clock: Some(Some(clock(1, 1))),
            ..base_encounter_image()
        })]
    );
}

// ---------------------------------------------------------------------------
// Memory is per view pair.
// ---------------------------------------------------------------------------

#[test]
fn fact_view_reports_its_epoch_and_clones_equal() {
    let facts = FactView::new(other_epoch());
    assert_eq!(facts.epoch(), other_epoch());
    assert_eq!(facts.clone(), facts);
    assert_ne!(facts, FactView::new(epoch()));
}

#[test]
fn c5_a_key_retired_behind_the_fact_view_is_forgotten() {
    // A snapshot applied straight to the interest view retires k2 with no op
    // and no call into the fact view. When k2 returns it has a new id, so the
    // remembered image (told under the old id) must not suppress the op.
    let fx = Keys::new(5);
    let scene = base(&fx);
    let (mut view, mut facts) = fresh();
    scene.run(&mut view, &mut facts).expect("first");

    let mut away = scene.clone();
    away.state[1].fields.presence = false;
    let state = ViewerContext {
        epoch: epoch(),
        tick: 10,
        interest: InterestSet { area: u(10) },
        facts: &away.state,
        despawned: &[],
        turn: None,
        turn_granted: false,
        resend_turn: false,
    };
    view.project_snapshot(&state).expect("snapshot retires k2");
    assert_eq!(view.net_id(fx.k(2)), None);

    let page = scene.run(&mut view, &mut facts).expect("k2 returns");
    assert_eq!(
        fact_ops(&page).last(),
        Some(&entity_facts(4, k2_image())),
        "the old image under id 2 must not suppress id 4"
    );
    assert_eq!(page.enters, 1);
}
