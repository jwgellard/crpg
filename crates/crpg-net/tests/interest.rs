//! Independent two-keys contract tests for T027b interest projection.
//!
//! Derived from `tasks/T027b.md` §"Exact contract" only, through the public
//! `crpg-net` API. Each test names the contract Behaviour items it proves and
//! the I-case it implements. Expected values are hand-written literals or
//! computed by the test-local [`Mirror`] plus the independent [`Oracle`]
//! (own id counter), never by production projection. Every rejection compares
//! the view against a pre-call clone. Each case has its valid control.

use std::collections::{BTreeMap, BTreeSet};

use crpg_core::{DeterministicRng, EntityId, GenerationalArena, Ulid};
use crpg_net::codec::{decode_delta, encode_delta};
use crpg_net::codec_transfer::{encode_body, plan_transfer};
use crpg_net::codec_v2;
use crpg_net::interest::{
    EntityFact, EntityKey, FieldMask, InterestError, InterestSet, InterestView, StateProjection,
    TurnState, ViewCursor, ViewerContext, MAX_PAGE_OPS,
};
use crpg_net::projection_v2::{project_events, EventCandidate};
use crpg_net::protocol::{
    DeltaFrame, DeltaOp, NetId, SessionEpoch, DELTA_TAG_ENTITY_ENTER, DELTA_TAG_HEALTH,
    DELTA_TAG_TURN, LANE_COMBAT, MAX_DELTA_OPS, MAX_VISIBLE_ENTITIES, PROTOCOL_VERSION,
};
use crpg_net::protocol_transfer::{HealthFact, SnapshotBody, TransferId, TurnFact};
use crpg_net::protocol_v2::{DeltaFrame as DeltaFrameV2, DeltaOp as DeltaOpV2};

// ---------------------------------------------------------------------------
// Small fixtures.
// ---------------------------------------------------------------------------

fn epoch(byte: u8) -> SessionEpoch {
    SessionEpoch([byte; 16])
}

fn net(raw: u64) -> NetId {
    NetId::new(raw).expect("fixture net id is nonzero")
}

fn area(n: u128) -> Ulid {
    Ulid::from_u128(n)
}

fn key(area_n: u128, entity: EntityId) -> EntityKey {
    EntityKey {
        area: area(area_n),
        entity,
    }
}

fn health(h: u32, m: u32, dead: bool) -> HealthFact {
    HealthFact {
        health: h,
        max_health: m,
        dead,
    }
}

fn mask(presence: bool, health_grant: bool) -> FieldMask {
    FieldMask {
        presence,
        health: health_grant,
    }
}

fn fact(
    k: EntityKey,
    presence: bool,
    health_grant: bool,
    hv: Option<HealthFact>,
    resend: bool,
) -> EntityFact {
    EntityFact {
        key: k,
        fields: mask(presence, health_grant),
        health: hv,
        resend_health: resend,
    }
}

fn turn_state(area_n: u128, active: Option<EntityId>, round: u64) -> TurnState {
    TurnState {
        area: area(area_n),
        active,
        round,
    }
}

#[allow(clippy::too_many_arguments)]
fn context<'a>(
    epoch_v: SessionEpoch,
    tick: u64,
    interest_area: u128,
    facts: &'a [EntityFact],
    despawned: &'a [EntityKey],
    turn: Option<TurnState>,
    turn_granted: bool,
    resend_turn: bool,
) -> ViewerContext<'a> {
    ViewerContext {
        epoch: epoch_v,
        tick,
        interest: InterestSet {
            area: area(interest_area),
        },
        facts,
        despawned,
        turn,
        turn_granted,
        resend_turn,
    }
}

fn arena_one() -> (GenerationalArena<()>, EntityId) {
    let mut arena = GenerationalArena::new();
    let id = arena.insert(());
    (arena, id)
}

fn arena_n(n: usize) -> (GenerationalArena<()>, Vec<EntityId>) {
    let mut arena = GenerationalArena::new();
    let mut out = Vec::new();
    for _ in 0..n {
        out.push(arena.insert(()));
    }
    (arena, out)
}

// ---------------------------------------------------------------------------
// Test-local Mirror: applies ops/bodies, fails loudly on protocol faults.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Mirror {
    live: BTreeMap<u64, Option<HealthFact>>,
    turn: Option<TurnFact>,
    seen: BTreeSet<u64>,
}

impl Mirror {
    fn new() -> Self {
        Self::default()
    }

    fn apply(&mut self, proj: &StateProjection) -> Result<(), String> {
        let ops = &proj.ops;
        if proj.enters > ops.len() {
            return Err("enters beyond ops".to_owned());
        }
        for (i, op) in ops.iter().enumerate() {
            let is_enter = matches!(op, DeltaOp::EntityEnter { .. });
            if i < proj.enters && !is_enter {
                return Err("non-enter inside enters slot".to_owned());
            }
            if i >= proj.enters && is_enter {
                return Err("enter outside enters slot".to_owned());
            }
        }
        for op in ops {
            match op {
                DeltaOp::EntityEnter { entity } => {
                    let raw = entity.get();
                    if self.live.contains_key(&raw) {
                        return Err("repeated enter".to_owned());
                    }
                    if self.seen.contains(&raw) {
                        return Err("reused id".to_owned());
                    }
                    self.seen.insert(raw);
                    self.live.insert(raw, None);
                }
                DeltaOp::Health {
                    entity,
                    health: h,
                    max_health: m,
                    dead,
                } => {
                    let raw = entity.get();
                    match self.live.get_mut(&raw) {
                        None => return Err("unknown id in Health".to_owned()),
                        Some(slot) => {
                            *slot = Some(HealthFact {
                                health: *h,
                                max_health: *m,
                                dead: *dead,
                            })
                        }
                    }
                }
                DeltaOp::Turn { active, round } => {
                    if let Some(a) = active {
                        if !self.live.contains_key(&a.get()) {
                            return Err("unknown id in Turn".to_owned());
                        }
                    }
                    self.turn = Some(TurnFact {
                        active: *active,
                        round: *round,
                    });
                }
                DeltaOp::EntityLeave { entity } | DeltaOp::Despawned { entity } => {
                    let raw = entity.get();
                    if self.live.remove(&raw).is_none() {
                        return Err("unknown or duplicate removal".to_owned());
                    }
                }
                DeltaOp::Spawned { .. } | DeltaOp::Died { .. } | DeltaOp::Receipt { .. } => {
                    return Err("notice owned by history/host".to_owned());
                }
            }
        }
        Ok(())
    }

    fn apply_body(&mut self, body: &SnapshotBody) -> Result<(), String> {
        let mut last: Option<u64> = None;
        for e in &body.entities {
            let raw = e.entity.get();
            match last {
                Some(prev) if raw <= prev => return Err("body not ascending".to_owned()),
                _ => last = Some(raw),
            }
        }
        if let Some(t) = &body.turn {
            if let Some(a) = t.active {
                if !body.entities.iter().any(|e| e.entity == a) {
                    return Err("turn active outside body".to_owned());
                }
            }
        }
        let mut next_live = BTreeMap::new();
        for e in &body.entities {
            let raw = e.entity.get();
            let was_live = self.live.contains_key(&raw);
            if self.seen.contains(&raw) && !was_live {
                return Err("reused id in snapshot".to_owned());
            }
            self.seen.insert(raw);
            next_live.insert(raw, e.health.clone());
        }
        self.live = next_live;
        self.turn = body.turn.clone();
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Independent oracle with its own id counter (never production projection).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct Oracle {
    next: u64,
    live: BTreeMap<EntityKey, NetId>,
    health: BTreeMap<u64, Option<HealthFact>>,
    turn: Option<TurnFact>,
}

impl Oracle {
    fn new() -> Self {
        Self {
            next: 1,
            live: BTreeMap::new(),
            health: BTreeMap::new(),
            turn: None,
        }
    }

    fn visible_keys(facts: &[EntityFact], interest: Ulid) -> Vec<EntityKey> {
        let mut out: Vec<EntityKey> = facts
            .iter()
            .filter(|f| f.key.area == interest && f.fields.presence)
            .map(|f| f.key)
            .collect();
        out.sort();
        out.dedup();
        out
    }

    fn rendered_health(facts: &[EntityFact], k: EntityKey) -> Option<HealthFact> {
        facts.iter().find(|f| f.key == k).and_then(|f| {
            if f.fields.health {
                f.health.clone()
            } else {
                None
            }
        })
    }

    fn rendered_turn(
        turn: Option<TurnState>,
        granted: bool,
        interest: Ulid,
        live: &BTreeMap<EntityKey, NetId>,
    ) -> Option<TurnFact> {
        if !granted {
            return None;
        }
        let t = turn?;
        if t.area != interest {
            return None;
        }
        let active = match t.active {
            None => None,
            Some(e) => {
                let k = EntityKey {
                    area: t.area,
                    entity: e,
                };
                live.get(&k).copied()
            }
        };
        Some(TurnFact {
            active,
            round: t.round,
        })
    }

    fn step_delta(
        &mut self,
        facts: &[EntityFact],
        interest: Ulid,
        turn: Option<TurnState>,
        granted: bool,
    ) -> Result<(), InterestError> {
        let visible = Self::visible_keys(facts, interest);
        let entering: Vec<EntityKey> = visible
            .iter()
            .copied()
            .filter(|k| !self.live.contains_key(k))
            .collect();
        if entering.len() as u64 > u64::MAX - self.next {
            return Err(InterestError::IdExhausted);
        }
        let removed: Vec<EntityKey> = self
            .live
            .keys()
            .copied()
            .filter(|k| !visible.contains(k))
            .collect();
        for k in &entering {
            let id = net(self.next);
            self.next += 1;
            self.live.insert(*k, id);
        }
        for k in &removed {
            if let Some(id) = self.live.remove(k) {
                self.health.remove(&id.get());
            }
        }
        // Delta mode never clears `Health` or `Turn` (contract item 6-7,
        // T046 N6): the client keeps the last disclosed value until replaced.
        for k in &visible {
            let id = self.live.get(k).copied().expect("visible mapped");
            let rendered = Self::rendered_health(facts, *k);
            match self.health.entry(id.get()) {
                std::collections::btree_map::Entry::Occupied(mut e) => {
                    if rendered.is_some() {
                        e.insert(rendered);
                    }
                }
                std::collections::btree_map::Entry::Vacant(e) => {
                    e.insert(rendered);
                }
            }
        }
        if let Some(rendered) = Self::rendered_turn(turn, granted, interest, &self.live) {
            self.turn = Some(rendered);
        }
        Ok(())
    }

    fn step_snapshot(
        &mut self,
        facts: &[EntityFact],
        interest: Ulid,
        turn: Option<TurnState>,
        granted: bool,
    ) -> Result<(), InterestError> {
        // Snapshot replaces the whole replica (contract items 12-13): the body
        // carries rendered health/turn exactly, including `None`.
        let visible = Self::visible_keys(facts, interest);
        let entering: Vec<EntityKey> = visible
            .iter()
            .copied()
            .filter(|k| !self.live.contains_key(k))
            .collect();
        if entering.len() as u64 > u64::MAX - self.next {
            return Err(InterestError::IdExhausted);
        }
        let removed: Vec<EntityKey> = self
            .live
            .keys()
            .copied()
            .filter(|k| !visible.contains(k))
            .collect();
        for k in &entering {
            let id = net(self.next);
            self.next += 1;
            self.live.insert(*k, id);
        }
        for k in &removed {
            if let Some(id) = self.live.remove(k) {
                self.health.remove(&id.get());
            }
        }
        for k in &visible {
            let id = self.live.get(k).copied().expect("visible mapped");
            self.health
                .insert(id.get(), Self::rendered_health(facts, *k));
        }
        self.turn = Self::rendered_turn(turn, granted, interest, &self.live);
        Ok(())
    }
}

fn assert_view_matches_oracle(view: &InterestView, oracle: &Oracle) {
    assert_eq!(view.len(), oracle.live.len());
    assert_eq!(view.is_empty(), oracle.live.is_empty());
    for (k, id) in &oracle.live {
        assert_eq!(view.net_id(*k), Some(*id), "net_id for {k:?}");
        assert_eq!(view.entity(*id), Some(*k), "entity for {id:?}");
    }
}

fn assert_mirror_matches_oracle(mirror: &Mirror, oracle: &Oracle) {
    assert_eq!(mirror.live.len(), oracle.live.len());
    for (k, id) in &oracle.live {
        let got = mirror.live.get(&id.get()).unwrap_or_else(|| {
            panic!("mirror missing id {:?} for key {k:?}", id.get());
        });
        let want = oracle.health.get(&id.get()).expect("oracle health");
        assert_eq!(got, want, "health for {k:?}");
    }
    assert_eq!(&mirror.turn, &oracle.turn);
}

// ---------------------------------------------------------------------------
// I1 (items 3, 10). Two viewers, two areas.
// ---------------------------------------------------------------------------

#[test]
fn i01_two_viewers_two_areas() {
    let ep = epoch(0x11);
    let (mut arena_x, ids_x) = arena_n(2);
    let e_y1 = arena_x.insert(());
    let e_y2 = arena_x.insert(());
    let x0 = ids_x[0];
    let x1 = ids_x[1];
    // One shared slice with rows in X and Y.
    let facts = vec![
        fact(key(1, x0), true, true, Some(health(8, 10, false)), false),
        fact(key(1, x1), true, false, None, false),
        fact(key(2, e_y1), true, true, Some(health(5, 9, false)), false),
        fact(key(2, e_y2), true, false, None, false),
    ];
    let empty: Vec<EntityKey> = vec![];
    let mut vx = InterestView::new(ep);
    let mut vy = InterestView::new(ep);
    let px = vx
        .project_delta(&context(ep, 1, 1, &facts, &empty, None, false, false))
        .expect("viewer X projects");
    let py = vy
        .project_delta(&context(ep, 1, 2, &facts, &empty, None, false, false))
        .expect("viewer Y projects");
    // Each viewer enters only its own rows, ids from 1.
    assert_eq!(px.enters, 2);
    assert_eq!(py.enters, 2);
    assert_eq!(vx.len(), 2);
    assert_eq!(vy.len(), 2);
    assert_eq!(vx.net_id(key(1, x0)), Some(net(1)));
    assert_eq!(vy.net_id(key(2, e_y1)), Some(net(1)));
    assert_eq!(vx.net_id(key(2, e_y1)), None);
    assert_eq!(vy.net_id(key(1, x0)), None);
    // Hidden rows consumed nothing: X has no Y mapping and vice versa.
    assert_eq!(px.ops.len(), 3); // 2 enters + 1 health (only health-granted row)
    assert_eq!(py.ops.len(), 3);

    // Control: move one X row to Y (same EntityId, new area key).
    let moved = vec![
        fact(key(1, x1), true, false, None, false),
        fact(key(2, x0), true, true, Some(health(8, 10, false)), false),
        fact(key(2, e_y1), true, true, Some(health(5, 9, false)), false),
        fact(key(2, e_y2), true, false, None, false),
    ];
    let qy = vy
        .project_delta(&context(ep, 2, 2, &moved, &empty, None, false, false))
        .expect("moved row enters Y");
    // Exactly one EntityEnter appears there (x0 in Y is new; others unchanged).
    let enters: Vec<&DeltaOp> = qy
        .ops
        .iter()
        .filter(|op| matches!(op, DeltaOp::EntityEnter { .. }))
        .collect();
    assert_eq!(enters.len(), 1);
    assert_eq!(enters[0], &DeltaOp::EntityEnter { entity: net(3) });
    // Viewer X now drops the moved row with exactly one removal.
    let qx = vx
        .project_delta(&context(ep, 2, 1, &moved, &empty, None, false, false))
        .expect("X drops moved row");
    assert_eq!(qx.ops, vec![DeltaOp::EntityLeave { entity: net(1) }]);
}

// ---------------------------------------------------------------------------
// I2 (D2, items 3, 9). Keys across areas and slot reuse.
// ---------------------------------------------------------------------------

#[test]
fn i02_equal_ids_across_areas_and_slot_reuse() {
    let ep = epoch(0x12);
    // Two arenas mint equal ids.
    let (_a1, id_a) = arena_one();
    let (_a2, id_b) = arena_one();
    assert_eq!(id_a, id_b);
    let kx = key(1, id_a);
    let ky = key(2, id_b);
    assert_ne!(kx, ky);
    let empty: Vec<EntityKey> = vec![];

    let mut view = InterestView::new(ep);
    let fx = vec![fact(kx, true, false, None, false)];
    let px = view
        .project_delta(&context(ep, 1, 1, &fx, &empty, None, false, false))
        .expect("X enters");
    assert_eq!(px.ops, vec![DeltaOp::EntityEnter { entity: net(1) }]);
    assert_eq!(view.net_id(kx), Some(net(1)));

    // Same viewer now in Y sees the equal-id row as a different key: leave + enter.
    let fy = vec![fact(ky, true, false, None, false)];
    let py = view
        .project_delta(&context(ep, 2, 2, &fy, &empty, None, false, false))
        .expect("move to Y");
    assert_eq!(
        py.ops,
        vec![
            DeltaOp::EntityEnter { entity: net(2) },
            DeltaOp::EntityLeave { entity: net(1) },
        ]
    );
    assert_eq!(view.net_id(kx), None);
    assert_eq!(view.entity(net(1)), None);
    assert_eq!(view.net_id(ky), Some(net(2)));

    // Slot reuse: remove then insert gives same index, higher generation.
    let mut arena = GenerationalArena::new();
    let first = arena.insert(());
    arena.remove(first);
    let second = arena.insert(());
    assert_eq!(first.index(), second.index());
    assert_ne!(first, second);
    let k_first = key(7, first);
    let k_second = key(7, second);
    let mut view2 = InterestView::new(ep);
    let f1 = vec![fact(k_first, true, false, None, false)];
    let p1 = view2
        .project_delta(&context(ep, 1, 7, &f1, &empty, None, false, false))
        .expect("first enters");
    assert_eq!(p1.ops, vec![DeltaOp::EntityEnter { entity: net(1) }]);
    let gone: Vec<EntityFact> = vec![];
    let p2 = view2
        .project_delta(&context(ep, 2, 7, &gone, &empty, None, false, false))
        .expect("first leaves");
    assert_eq!(p2.ops, vec![DeltaOp::EntityLeave { entity: net(1) }]);
    let f2 = vec![fact(k_second, true, false, None, false)];
    let p3 = view2
        .project_delta(&context(ep, 3, 7, &f2, &empty, None, false, false))
        .expect("reused slot enters");
    assert_eq!(p3.ops, vec![DeltaOp::EntityEnter { entity: net(2) }]);
    assert_eq!(view2.entity(net(1)), None);
}

// ---------------------------------------------------------------------------
// I3 (items 6, 10, D4). Field-by-field suppression.
// ---------------------------------------------------------------------------

#[test]
fn i03_field_by_field_suppression() {
    let ep = epoch(0x13);
    let (_arena, eid) = arena_one();
    let k = key(1, eid);
    let secret = Some(health(7, 10, false));
    let turn = Some(turn_state(1, Some(eid), 4));
    let empty: Vec<EntityKey> = vec![];

    // presence=false denies state even with grants and values; turn is kept
    // out of rendering here so the page is empty (Turn is covered below).
    for health_grant in [false, true] {
        let mut v = InterestView::new(ep);
        let f = vec![fact(k, false, health_grant, secret.clone(), false)];
        let p = v
            .project_delta(&context(ep, 1, 1, &f, &empty, None, false, false))
            .expect("hidden projects");
        assert!(p.ops.is_empty(), "hidden must be empty");
        assert_eq!(v.len(), 0);
    }
    // Hidden plus a granted turn still renders Turn with active None (item 7).
    {
        let mut v = InterestView::new(ep);
        let f = vec![fact(k, false, false, secret.clone(), false)];
        let p = v
            .project_delta(&context(ep, 1, 1, &f, &empty, turn, true, false))
            .expect("hidden turn renders");
        assert_eq!(
            p.ops,
            vec![DeltaOp::Turn {
                active: None,
                round: 4
            }]
        );
    }
    // FieldMask::default shows nothing (no turn rendered here).
    {
        let mut v = InterestView::new(ep);
        let f = vec![EntityFact {
            key: k,
            fields: FieldMask::default(),
            health: secret.clone(),
            resend_health: false,
        }];
        let p = v
            .project_delta(&context(ep, 1, 1, &f, &empty, None, false, false))
            .expect("default mask");
        assert!(p.ops.is_empty());
        assert_eq!(v.len(), 0);
    }
    // presence=true, health=false, turn granted=false: enter only.
    {
        let mut v = InterestView::new(ep);
        let f = vec![fact(k, true, false, secret.clone(), false)];
        let p = v
            .project_delta(&context(ep, 1, 1, &f, &empty, turn, false, false))
            .expect("enter only");
        assert_eq!(p.ops, vec![DeltaOp::EntityEnter { entity: net(1) }]);
    }
    // Control per suppression: setting the health bit adds exactly the Health
    // op at its fixed index (after enters, before Turn).
    {
        let mut v = InterestView::new(ep);
        let f = vec![fact(k, true, true, secret.clone(), false)];
        let p = v
            .project_delta(&context(ep, 1, 1, &f, &empty, turn, false, false))
            .expect("enter plus health");
        assert_eq!(
            p.ops,
            vec![
                DeltaOp::EntityEnter { entity: net(1) },
                DeltaOp::Health {
                    entity: net(1),
                    health: 7,
                    max_health: 10,
                    dead: false
                },
            ]
        );
    }
    // Control: setting the turn grant adds exactly the Turn op at its fixed
    // index (after Health).
    {
        let mut v = InterestView::new(ep);
        let f = vec![fact(k, true, true, secret.clone(), false)];
        let p = v
            .project_delta(&context(ep, 1, 1, &f, &empty, turn, true, false))
            .expect("enter health turn");
        assert_eq!(
            p.ops,
            vec![
                DeltaOp::EntityEnter { entity: net(1) },
                DeltaOp::Health {
                    entity: net(1),
                    health: 7,
                    max_health: 10,
                    dead: false
                },
                DeltaOp::Turn {
                    active: Some(net(1)),
                    round: 4
                },
            ]
        );
    }
    // health grant with None value emits no Health (needs a value).
    {
        let mut v = InterestView::new(ep);
        let f = vec![fact(k, true, true, None, false)];
        let p = v
            .project_delta(&context(ep, 1, 1, &f, &empty, None, false, false))
            .expect("no value");
        assert_eq!(p.ops, vec![DeltaOp::EntityEnter { entity: net(1) }]);
    }
    // All 8 combinations of presence, health grant and `turn_granted` over one
    // row with a distinctive secret health, each with its exact ops.
    for (presence, hgrant, tgrant) in [
        (false, false, false),
        (false, false, true),
        (false, true, false),
        (false, true, true),
        (true, false, false),
        (true, false, true),
        (true, true, false),
        (true, true, true),
    ] {
        let mut v = InterestView::new(ep);
        let f = vec![fact(k, presence, hgrant, secret.clone(), false)];
        let p = v
            .project_delta(&context(ep, 1, 1, &f, &empty, turn, tgrant, false))
            .expect("combo projects");
        let want: Vec<DeltaOp> = match (presence, hgrant, tgrant) {
            (false, _, false) => vec![],
            (false, _, true) => vec![DeltaOp::Turn {
                active: None,
                round: 4,
            }],
            (true, false, false) => vec![DeltaOp::EntityEnter { entity: net(1) }],
            (true, false, true) => vec![
                DeltaOp::EntityEnter { entity: net(1) },
                DeltaOp::Turn {
                    active: Some(net(1)),
                    round: 4,
                },
            ],
            (true, true, false) => vec![
                DeltaOp::EntityEnter { entity: net(1) },
                DeltaOp::Health {
                    entity: net(1),
                    health: 7,
                    max_health: 10,
                    dead: false,
                },
            ],
            (true, true, true) => vec![
                DeltaOp::EntityEnter { entity: net(1) },
                DeltaOp::Health {
                    entity: net(1),
                    health: 7,
                    max_health: 10,
                    dead: false,
                },
                DeltaOp::Turn {
                    active: Some(net(1)),
                    round: 4,
                },
            ],
        };
        assert_eq!(p.ops, want, "combo {presence}/{hgrant}/{tgrant}");
    }
}

// ---------------------------------------------------------------------------
// I4 (items 4, 5, 6, 9). Enter, update, leave, re-enter; order invariance.
// ---------------------------------------------------------------------------

#[test]
fn i04_enter_update_leave_reenter() {
    let ep = epoch(0x14);
    let (_arena, e0) = arena_one();
    let (_arena2, e1) = arena_one();
    // Distinct ids: use two arenas sequentially would alias, so build from one.
    let (_arena, ids) = arena_n(3);
    let a = ids[0];
    let b = ids[1];
    let c = ids[2];
    let _ = (e0, e1, _arena);
    let ka = key(1, a);
    let kb = key(1, b);
    let kc = key(1, c);
    let hv = |h| Some(health(h, 10, false));
    let ordered = vec![
        fact(ka, true, true, hv(1), false),
        fact(kb, true, true, hv(2), false),
        fact(kc, true, true, hv(3), false),
    ];
    let shuffled = vec![ordered[2].clone(), ordered[0].clone(), ordered[1].clone()];
    let empty: Vec<EntityKey> = vec![];
    let mut v1 = InterestView::new(ep);
    let mut v2 = InterestView::new(ep);
    let p1 = v1
        .project_delta(&context(ep, 1, 1, &ordered, &empty, None, false, false))
        .expect("ordered");
    let p2 = v2
        .project_delta(&context(ep, 1, 1, &shuffled, &empty, None, false, false))
        .expect("shuffled");
    // Input order never matters: shuffled gives the literal page.
    assert_eq!(p1, p2);
    assert_eq!(p1.enters, 3);
    assert_eq!(
        p1.ops,
        vec![
            DeltaOp::EntityEnter { entity: net(1) },
            DeltaOp::EntityEnter { entity: net(2) },
            DeltaOp::EntityEnter { entity: net(3) },
            DeltaOp::Health {
                entity: net(1),
                health: 1,
                max_health: 10,
                dead: false
            },
            DeltaOp::Health {
                entity: net(2),
                health: 2,
                max_health: 10,
                dead: false
            },
            DeltaOp::Health {
                entity: net(3),
                health: 3,
                max_health: 10,
                dead: false
            },
        ]
    );
    // Minting is ascending EntityKey order, once each; check mapping order.
    let mut sorted = [ka, kb, kc];
    sorted.sort();
    assert_eq!(v1.net_id(sorted[0]), Some(net(1)));
    assert_eq!(v1.net_id(sorted[1]), Some(net(2)));
    assert_eq!(v1.net_id(sorted[2]), Some(net(3)));
    // No Spawned/Died/Receipt in any delta page.
    for op in &p1.ops {
        assert!(!matches!(
            op,
            DeltaOp::Spawned { .. } | DeltaOp::Died { .. } | DeltaOp::Receipt { .. }
        ));
    }
    // One change gives one Health.
    let updated = vec![
        fact(ka, true, true, hv(1), false),
        fact(kb, true, true, hv(9), false),
        fact(kc, true, true, hv(3), false),
    ];
    let pu = v1
        .project_delta(&context(ep, 2, 1, &updated, &empty, None, false, false))
        .expect("one change");
    assert_eq!(
        pu.ops,
        vec![DeltaOp::Health {
            entity: net(2),
            health: 9,
            max_health: 10,
            dead: false
        }]
    );
    // Leave retires: mapping gone for that key and id.
    let left = vec![
        fact(ka, true, true, hv(1), false),
        fact(kc, true, true, hv(3), false),
    ];
    let pl = v1
        .project_delta(&context(ep, 3, 1, &left, &empty, None, false, false))
        .expect("leave");
    assert_eq!(pl.ops, vec![DeltaOp::EntityLeave { entity: net(2) }]);
    assert_eq!(v1.net_id(kb), None);
    assert_eq!(v1.entity(net(2)), None);
    // Re-entry takes a new id; the old id stays None.
    let back = vec![
        fact(ka, true, true, hv(1), false),
        fact(kb, true, true, hv(2), false),
        fact(kc, true, true, hv(3), false),
    ];
    let pr = v1
        .project_delta(&context(ep, 4, 1, &back, &empty, None, false, false))
        .expect("re-enter");
    assert_eq!(
        pr.ops,
        vec![
            DeltaOp::EntityEnter { entity: net(4) },
            DeltaOp::Health {
                entity: net(4),
                health: 2,
                max_health: 10,
                dead: false
            },
        ]
    );
    assert_eq!(v1.net_id(kb), Some(net(4)));
    assert_eq!(v1.entity(net(2)), None);
}

// ---------------------------------------------------------------------------
// I5 (item 10). Hidden spawn consumes nothing.
// ---------------------------------------------------------------------------

#[test]
fn i05_hidden_spawn_consumes_nothing() {
    let ep = epoch(0x15);
    let (_arena, ids) = arena_n(2);
    let hidden_id = ids[0];
    let next_id = ids[1];
    let _ = _arena;
    let kh = key(1, hidden_id);
    let kv = key(1, next_id);
    let empty: Vec<EntityKey> = vec![];
    let mut view = InterestView::new(ep);
    // Hidden row emits nothing.
    let f_hidden = vec![fact(kh, false, false, Some(health(7, 7, false)), false)];
    let p = view
        .project_delta(&context(ep, 1, 1, &f_hidden, &empty, None, false, false))
        .expect("hidden");
    assert!(p.ops.is_empty());
    assert_eq!(view.len(), 0);
    assert_eq!(view.net_id(kh), None);
    // A hidden Spawned candidate yields nothing.
    let spawned_hidden = EventCandidate::Spawned {
        entity: view.net_id(kh),
        disclose: true,
    };
    assert_eq!(
        project_events(std::slice::from_ref(&spawned_hidden)).expect("projects"),
        vec![]
    );
    // The next visible entrant takes the id it would have taken anyway (1).
    let f_visible = vec![fact(kv, true, true, Some(health(3, 10, false)), false)];
    let q = view
        .project_delta(&context(ep, 2, 1, &f_visible, &empty, None, false, false))
        .expect("visible");
    assert_eq!(q.enters, 1);
    assert_eq!(view.net_id(kv), Some(net(1)));
    // Control: with presence the page has the enter and the same candidate
    // yields Spawned, placed at enters.
    let spawned = EventCandidate::Spawned {
        entity: view.net_id(kv),
        disclose: true,
    };
    let history = project_events(std::slice::from_ref(&spawned)).expect("projects");
    assert_eq!(
        history,
        vec![DeltaOpV2::Legacy(DeltaOp::Spawned { entity: net(1) })]
    );
    assert_eq!(q.enters, 1);
    assert!(matches!(q.ops[0], DeltaOp::EntityEnter { .. }));
}

// ---------------------------------------------------------------------------
// I6 (item 11, D5). Death is not removal.
// ---------------------------------------------------------------------------

#[test]
fn i06_death_stays_mapped() {
    let ep = epoch(0x16);
    let (_arena, eid) = arena_one();
    let k = key(1, eid);
    let empty: Vec<EntityKey> = vec![];
    let mut view = InterestView::new(ep);
    let f = vec![fact(k, true, true, Some(health(0, 10, true)), false)];
    let p = view
        .project_delta(&context(ep, 1, 1, &f, &empty, None, false, false))
        .expect("dead projects");
    // Dead row stays mapped with no removal op; Health carries dead:true.
    assert_eq!(view.len(), 1);
    assert_eq!(view.net_id(k), Some(net(1)));
    assert_eq!(
        p.ops,
        vec![
            DeltaOp::EntityEnter { entity: net(1) },
            DeltaOp::Health {
                entity: net(1),
                health: 0,
                max_health: 10,
                dead: true
            },
        ]
    );
    // Died needs both the mapping and disclose.
    let died = EventCandidate::Died {
        entity: view.net_id(k),
        disclose: true,
    };
    assert_eq!(
        project_events(std::slice::from_ref(&died)).expect("projects"),
        vec![DeltaOpV2::Legacy(DeltaOp::Died { entity: net(1) })]
    );
    // A hidden row yields nothing.
    let hidden_died = EventCandidate::Died {
        entity: None,
        disclose: true,
    };
    assert_eq!(
        project_events(std::slice::from_ref(&hidden_died)).expect("projects"),
        vec![]
    );
    let undisclosed = EventCandidate::Died {
        entity: view.net_id(k),
        disclose: false,
    };
    assert_eq!(
        project_events(std::slice::from_ref(&undisclosed)).expect("projects"),
        vec![]
    );
    // Control: mapped and disclosed yields Died (above); Health without a
    // disclose flag still shows dead:true and no Died (undisclosed above).
}

// ---------------------------------------------------------------------------
// I7 (item 8). Despawn vs leave.
// ---------------------------------------------------------------------------

#[test]
fn i07_despawn_leave_equivalence() {
    let ep = epoch(0x17);
    // Removal of a live key `k` (viewer area 1) with explicit facts/despawned.
    let run_case = |facts: Vec<EntityFact>, despawned: Vec<EntityKey>| {
        let (_arena, ids) = arena_n(2);
        let _ = _arena;
        // arena_n(2) mints two distinct ids; use the first for the live key so
        // every case mints the same EntityId and thus the same NetId 1.
        let k = key(1, ids[0]);
        let other_area = key(2, ids[0]);
        let _ = other_area;
        let mut v = InterestView::new(ep);
        let f = vec![fact(k, true, false, None, false)];
        let empty: Vec<EntityKey> = vec![];
        v.project_delta(&context(ep, 1, 1, &f, &empty, None, false, false))
            .expect("enter");
        v.project_delta(&context(ep, 2, 1, &facts, &despawned, None, false, false))
            .expect("remove")
    };

    // A still-present `presence: false` row, unlisted.
    let (_arena_p, ids_p) = arena_n(2);
    let _ = _arena_p;
    let kp = key(1, ids_p[0]);
    let still_present = vec![fact(kp, false, false, None, false)];
    // An area move: the same entity now has a row in area 2 only.
    let moved = vec![fact(key(2, ids_p[0]), true, false, None, false)];
    // A presence revoke: the row is gone entirely.
    let gone: Vec<EntityFact> = vec![];

    // Hidden despawn (still-present, unlisted), area move and presence revoke
    // give equal ops: exactly one EntityLeave, carrying no cause.
    let leave_present = run_case(still_present.clone(), vec![]);
    let leave_moved = run_case(moved, vec![]);
    let leave_gone = run_case(gone.clone(), vec![]);
    for page in [&leave_present, &leave_moved, &leave_gone] {
        assert_eq!(page.ops.len(), 1);
        assert!(matches!(page.ops[0], DeltaOp::EntityLeave { .. }));
    }
    assert_eq!(leave_present.ops, leave_moved.ops);
    assert_eq!(leave_present.ops, leave_gone.ops);

    // The same still-present row, listed in despawned, gives Despawned; so
    // does the gone row listed. Both are exactly one Despawned.
    let despawn_present = run_case(still_present, vec![kp]);
    let despawn_gone = run_case(gone, vec![kp]);
    assert_eq!(
        despawn_present.ops,
        vec![DeltaOp::Despawned { entity: net(1) }]
    );
    assert_eq!(despawn_present.ops, despawn_gone.ops);

    // Control: listing the key in despawned gives Despawned, no EntityLeave.
    {
        let (_arena, ids) = arena_n(1);
        let _ = _arena;
        let k = key(1, ids[0]);
        let mut v = InterestView::new(ep);
        let f = vec![fact(k, true, false, None, false)];
        let empty: Vec<EntityKey> = vec![];
        v.project_delta(&context(ep, 1, 1, &f, &empty, None, false, false))
            .expect("enter");
        let gone: Vec<EntityFact> = vec![];
        let p = v
            .project_delta(&context(ep, 2, 1, &gone, &[k], None, false, false))
            .expect("despawn");
        assert_eq!(p.ops, vec![DeltaOp::Despawned { entity: net(1) }]);
    }
    // A listed key that is still visible has no effect.
    {
        let (_arena, ids) = arena_n(1);
        let _ = _arena;
        let k = key(1, ids[0]);
        let mut v = InterestView::new(ep);
        let f = vec![fact(k, true, false, None, false)];
        let empty: Vec<EntityKey> = vec![];
        v.project_delta(&context(ep, 1, 1, &f, &empty, None, false, false))
            .expect("enter");
        let p = v
            .project_delta(&context(ep, 2, 1, &f, &[k], None, false, false))
            .expect("listed but visible");
        assert!(p.ops.is_empty());
        assert_eq!(v.len(), 1);
    }
    // A listed key that was never live emits nothing.
    {
        let (_arena, ids) = arena_n(1);
        let _ = _arena;
        let k = key(9, ids[0]);
        let mut v = InterestView::new(ep);
        let gone: Vec<EntityFact> = vec![];
        let p = v
            .project_delta(&context(ep, 1, 1, &gone, &[k], None, false, false))
            .expect("never live");
        assert!(p.ops.is_empty());
    }
    // A listed hidden non-live key emits nothing.
    {
        let (_arena, ids) = arena_n(1);
        let _ = _arena;
        let k = key(1, ids[0]);
        let mut v = InterestView::new(ep);
        let hidden = vec![fact(k, false, false, None, false)];
        let p = v
            .project_delta(&context(ep, 1, 1, &hidden, &[k], None, false, false))
            .expect("hidden listed");
        assert!(p.ops.is_empty());
        assert_eq!(v.len(), 0);
    }
}

// ---------------------------------------------------------------------------
// I8 (item 9). Same-tick revoke and request.
// ---------------------------------------------------------------------------

#[test]
fn i08_retired_ids_do_not_resolve() {
    let ep = epoch(0x18);
    let (mut arena, ids) = arena_n(2);
    let keep = ids[0];
    let drop_me = ids[1];
    let k_keep = key(1, keep);
    let k_drop = key(1, drop_me);
    let empty: Vec<EntityKey> = vec![];
    let mut view = InterestView::new(ep);
    let f = vec![
        fact(k_keep, true, false, None, false),
        fact(k_drop, true, false, None, false),
    ];
    view.project_delta(&context(ep, 1, 1, &f, &empty, None, false, false))
        .expect("enter both");
    let mut sorted = [k_keep, k_drop];
    sorted.sort();
    let dropped_net = view.net_id(k_drop).expect("dropped mapped");
    // Project a removal, then with no other call resolve through the
    // test-local copy of host resolution (view.entity).
    let remain = vec![fact(k_keep, true, false, None, false)];
    view.project_delta(&context(ep, 1, 1, &remain, &empty, None, false, false))
        .expect("remove one");
    // Retired denies.
    assert_eq!(view.entity(dropped_net), None);
    assert_eq!(view.net_id(k_drop), None);
    // Hidden, foreign and never-issued ids deny identically.
    assert_eq!(view.entity(net(999_999)), None);
    let fresh = arena.insert(());
    assert_eq!(view.net_id(key(1, fresh)), None);
    assert_eq!(view.net_id(key(2, keep)), None);
    // Control: a still-visible id resolves.
    let live_net = view.net_id(k_keep).expect("live mapped");
    assert_eq!(view.entity(live_net), Some(k_keep));
}

// ---------------------------------------------------------------------------
// I9 (item 7, D9). Turn projection.
// ---------------------------------------------------------------------------

#[test]
fn i09_turn_projection() {
    let ep = epoch(0x19);
    let (_arena, ids) = arena_n(3);
    let a = ids[0];
    let b = ids[1];
    let c = ids[2];
    let _ = _arena;
    let ka = key(1, a);
    let kb = key(1, b);
    let _kc = key(1, c);
    let empty: Vec<EntityKey> = vec![];

    // Visible active.
    let mut v = InterestView::new(ep);
    let f = vec![
        fact(ka, true, true, Some(health(10, 10, false)), false),
        fact(kb, true, true, Some(health(9, 10, false)), false),
    ];
    let t = Some(turn_state(1, Some(a), 2));
    let p = v
        .project_delta(&context(ep, 1, 1, &f, &empty, t, true, false))
        .expect("visible active");
    assert_eq!(
        p.ops.last(),
        Some(&DeltaOp::Turn {
            active: Some(net(1)),
            round: 2
        })
    );
    // Hidden active renders active:None.
    let t_hidden = Some(turn_state(1, Some(c), 3));
    let q = v
        .project_delta(&context(ep, 2, 1, &f, &empty, t_hidden, true, false))
        .expect("hidden active");
    assert_eq!(
        q.ops,
        vec![DeltaOp::Turn {
            active: None,
            round: 3
        }]
    );
    // Active entering this page takes its new id.
    let mut v2 = InterestView::new(ep);
    let f2 = vec![fact(ka, true, false, None, false)];
    let t2 = Some(turn_state(1, Some(a), 7));
    let p2 = v2
        .project_delta(&context(ep, 1, 1, &f2, &empty, t2, true, false))
        .expect("entering active");
    assert_eq!(
        p2.ops,
        vec![
            DeltaOp::EntityEnter { entity: net(1) },
            DeltaOp::Turn {
                active: Some(net(1)),
                round: 7
            },
        ]
    );
    // Active leaving this page: Turn is None and precedes the removal.
    let gone: Vec<EntityFact> = vec![];
    let t3 = Some(turn_state(1, Some(a), 8));
    let p3 = v2
        .project_delta(&context(ep, 2, 1, &gone, &empty, t3, true, false))
        .expect("leaving active");
    assert_eq!(
        p3.ops,
        vec![
            DeltaOp::Turn {
                active: None,
                round: 8
            },
            DeltaOp::EntityLeave { entity: net(1) },
        ]
    );
    // Foreign area: no op.
    let mut v3 = InterestView::new(ep);
    let t_foreign = Some(turn_state(2, Some(a), 1));
    let pf = v3
        .project_delta(&context(ep, 1, 1, &f, &empty, t_foreign, true, false))
        .expect("foreign turn");
    assert!(!pf.ops.iter().any(|op| matches!(op, DeltaOp::Turn { .. })));
    // No grant: no op. Control: the grant makes the op appear.
    let mut v4 = InterestView::new(ep);
    let pungranted = v4
        .project_delta(&context(ep, 1, 1, &f, &empty, t, false, false))
        .expect("ungranted");
    assert!(!pungranted
        .ops
        .iter()
        .any(|op| matches!(op, DeltaOp::Turn { .. })));
    let pgranted = v4
        .project_delta(&context(ep, 2, 1, &f, &empty, t, true, false))
        .expect("granted");
    assert_eq!(pgranted.ops.len(), 1);
    assert!(matches!(pgranted.ops[0], DeltaOp::Turn { .. }));
    // active:None renders Turn with None.
    let mut v5 = InterestView::new(ep);
    let t_none = Some(turn_state(1, None, 5));
    let pn = v5
        .project_delta(&context(ep, 1, 1, &f, &empty, t_none, true, false))
        .expect("none active");
    assert!(pn.ops.iter().any(|op| matches!(
        op,
        DeltaOp::Turn {
            active: None,
            round: 5
        }
    )));
    // An unchanged turn emits nothing.
    let repeat = v5
        .project_delta(&context(ep, 2, 1, &f, &empty, t_none, true, false))
        .expect("unchanged");
    assert!(repeat.ops.is_empty());
}

// ---------------------------------------------------------------------------
// I10 (items 6, 7, D12). Resend flags.
// ---------------------------------------------------------------------------

#[test]
fn i10_resend_reemits_once() {
    let ep = epoch(0x1A);
    let (_arena, ids) = arena_n(2);
    let a = ids[0];
    let b = ids[1];
    let _ = _arena;
    let ka = key(1, a);
    let kb = key(1, b);
    let empty: Vec<EntityKey> = vec![];
    let mut v = InterestView::new(ep);
    let base = vec![
        fact(ka, true, true, Some(health(4, 10, false)), false),
        fact(kb, true, true, Some(health(6, 10, false)), false),
    ];
    let t = Some(turn_state(1, Some(a), 1));
    v.project_delta(&context(ep, 1, 1, &base, &empty, t, true, false))
        .expect("base");
    // Unchanged without flags: empty.
    let same = v
        .project_delta(&context(ep, 2, 1, &base, &empty, t, true, false))
        .expect("unchanged");
    assert!(same.ops.is_empty());
    // Resend re-emits unchanged values, once, for that page only.
    let mut resent = base.clone();
    for row in &mut resent {
        row.resend_health = true;
    }
    let pr = v
        .project_delta(&context(ep, 3, 1, &resent, &empty, t, true, true))
        .expect("resend");
    assert_eq!(
        pr.ops,
        vec![
            DeltaOp::Health {
                entity: net(1),
                health: 4,
                max_health: 10,
                dead: false
            },
            DeltaOp::Health {
                entity: net(2),
                health: 6,
                max_health: 10,
                dead: false
            },
            DeltaOp::Turn {
                active: Some(net(1)),
                round: 1
            },
        ]
    );
    // Next page without flags is empty again.
    let after = v
        .project_delta(&context(ep, 4, 1, &base, &empty, t, true, false))
        .expect("after resend");
    assert!(after.ops.is_empty());
    // Clear a grant, then re-grant it with the same value: this must re-emit
    // Health (and Turn). A cleared grant emits nothing and retracts nothing.
    let cleared = vec![
        fact(ka, true, false, Some(health(4, 10, false)), false),
        fact(kb, true, true, Some(health(6, 10, false)), false),
    ];
    let pc = v
        .project_delta(&context(ep, 5, 1, &cleared, &empty, t, false, false))
        .expect("clear grants");
    assert!(pc.ops.is_empty());
    assert_eq!(v.len(), 2);
    let regranted = v
        .project_delta(&context(ep, 6, 1, &base, &empty, t, true, false))
        .expect("re-grant same values");
    assert_eq!(
        regranted.ops,
        vec![
            DeltaOp::Health {
                entity: net(1),
                health: 4,
                max_health: 10,
                dead: false
            },
            DeltaOp::Turn {
                active: Some(net(1)),
                round: 1
            },
        ]
    );
    // Health is emitted after an enter with the grant off and then granted.
    {
        let mut w = InterestView::new(ep);
        let enter_off = vec![fact(ka, true, false, Some(health(4, 10, false)), false)];
        let pe = w
            .project_delta(&context(ep, 1, 1, &enter_off, &empty, None, false, false))
            .expect("enter grant off");
        assert_eq!(pe.ops, vec![DeltaOp::EntityEnter { entity: net(1) }]);
        let grant_on = vec![fact(ka, true, true, Some(health(4, 10, false)), false)];
        let pg2 = w
            .project_delta(&context(ep, 2, 1, &grant_on, &empty, None, false, false))
            .expect("grant after enter");
        assert_eq!(
            pg2.ops,
            vec![DeltaOp::Health {
                entity: net(1),
                health: 4,
                max_health: 10,
                dead: false
            }]
        );
    }
    // Resend never overrides a missing grant.
    let mut nogrant = vec![
        fact(ka, true, false, Some(health(4, 10, false)), true),
        fact(kb, true, false, Some(health(6, 10, false)), true),
    ];
    let _ = &mut nogrant;
    let pg = v
        .project_delta(&context(ep, 7, 1, &nogrant, &empty, t, false, true))
        .expect("resend without grant");
    assert!(pg.ops.is_empty());
}

// ---------------------------------------------------------------------------
// I11 (item 5, D12). T022 layout parity and literal snapshot body.
// ---------------------------------------------------------------------------

#[test]
fn i11_t022_layout_parity() {
    let ep = epoch(0x1B);
    let (_arena, ids) = arena_n(3);
    let a = ids[0];
    let b = ids[1];
    let cc = ids[2];
    let _ = _arena;
    let ka = key(1, a);
    let kb = key(1, b);
    let kc = key(1, cc);
    let empty: Vec<EntityKey> = vec![];
    let mut v = InterestView::new(ep);
    let facts = vec![
        fact(kb, true, true, Some(health(6, 10, false)), false),
        fact(ka, true, true, Some(health(4, 12, false)), false),
    ];
    let t = Some(turn_state(1, Some(a), 2));
    let p = v
        .project_delta(&context(ep, 10, 1, &facts, &empty, t, true, false))
        .expect("fresh full state");
    // Literal fresh page: enters, healths, turn (ascending NetId).
    let mut ordered = [ka, kb];
    ordered.sort();
    let first = v.net_id(ordered[0]).expect("first");
    let second = v.net_id(ordered[1]).expect("second");
    assert_eq!((first, second), (net(1), net(2)));
    let active_net = if ordered[0] == ka { net(1) } else { net(2) };
    let (h_first, m_first) = if ordered[0] == ka { (4, 12) } else { (6, 10) };
    let (h_second, m_second) = if ordered[1] == ka { (4, 12) } else { (6, 10) };
    assert_eq!(p.enters, 2);
    assert_eq!(
        p.ops,
        vec![
            DeltaOp::EntityEnter { entity: net(1) },
            DeltaOp::EntityEnter { entity: net(2) },
            DeltaOp::Health {
                entity: net(1),
                health: h_first,
                max_health: m_first,
                dead: false
            },
            DeltaOp::Health {
                entity: net(2),
                health: h_second,
                max_health: m_second,
                dead: false
            },
            DeltaOp::Turn {
                active: Some(active_net),
                round: 2
            },
        ]
    );
    // Widening: change one health and the round, with a page that contains
    // enters (kc enters here). Layout stays enters, healths, turn.
    let facts2 = vec![
        fact(ka, true, true, Some(health(3, 12, false)), false),
        fact(kb, true, true, Some(health(6, 10, false)), false),
        fact(kc, true, true, Some(health(9, 9, false)), false),
    ];
    let t2 = Some(turn_state(1, Some(b), 3));
    let active2 = v.net_id(if ordered.contains(&ka) { kb } else { ka });
    let _ = active2;
    let w = v
        .project_delta(&context(ep, 11, 1, &facts2, &empty, t2, true, false))
        .expect("widening");
    // Widening order is the same layout: enters, healths, then turn.
    let want_active = v.net_id(kb).expect("kb still live");
    let want_enter = v.net_id(kc).expect("kc entered");
    assert_eq!(w.enters, 1);
    assert_eq!(want_enter, net(3));
    assert_eq!(
        w.ops,
        vec![
            DeltaOp::EntityEnter { entity: net(3) },
            DeltaOp::Health {
                entity: v.net_id(ka).expect("ka"),
                health: 3,
                max_health: 12,
                dead: false
            },
            DeltaOp::Health {
                entity: net(3),
                health: 9,
                max_health: 9,
                dead: false
            },
            DeltaOp::Turn {
                active: Some(want_active),
                round: 3
            },
        ]
    );
    // The body equals a literal snapshot_body.
    let snap = v
        .project_snapshot(&context(ep, 11, 1, &facts2, &empty, t2, true, false))
        .expect("snapshot");
    let body = snap.body;
    assert_eq!(body.entities.len(), 3);
    assert_eq!(body.entities[0].entity, net(1));
    assert_eq!(body.entities[1].entity, net(2));
    assert_eq!(body.entities[2].entity, net(3));
    assert_eq!(
        body.turn,
        Some(TurnFact {
            active: want_active.into(),
            round: 3
        })
    );
}

// ---------------------------------------------------------------------------
// I12 (items 12, 13, 14). Snapshot, cutover and cursor.
// ---------------------------------------------------------------------------

#[test]
fn i12_snapshot_cutover_cursor() {
    let ep = epoch(0x1C);
    let (_arena, ids) = arena_n(4);
    let a = ids[0];
    let b = ids[1];
    let c = ids[2];
    let d = ids[3];
    let _ = _arena;
    let ka = key(1, a);
    let kb = key(1, b);
    let kc = key(1, c);
    let kd = key(1, d);
    let empty: Vec<EntityKey> = vec![];
    let mut v = InterestView::new(ep);
    assert_eq!(v.cursor(), ViewCursor::default());
    let facts = vec![
        fact(ka, true, true, Some(health(4, 10, false)), false),
        fact(kb, true, false, Some(health(9, 9, false)), false),
        fact(kc, true, true, Some(health(1, 5, false)), false),
    ];
    let t = Some(turn_state(1, Some(b), 2));
    let p1 = v
        .project_delta(&context(ep, 10, 1, &facts, &empty, t, true, false))
        .expect("base delta");
    assert!(!p1.ops.is_empty());
    let after_delta = v.cursor();
    assert_eq!(after_delta.tick, 10);
    assert_eq!(after_delta.revision, 1);
    // Mid-epoch snapshot drops kc, changes ka's health, enters kd, keeps live
    // ids, health only where granted, hidden active renders None.
    let snap_facts = vec![
        fact(ka, true, true, Some(health(5, 10, false)), false),
        fact(kb, true, false, Some(health(9, 9, false)), false),
        fact(kd, true, true, Some(health(2, 8, false)), false),
    ];
    let t_hidden = Some(turn_state(1, Some(c), 2));
    let snap = v
        .project_snapshot(&context(
            ep,
            11,
            1,
            &snap_facts,
            &empty,
            t_hidden,
            true,
            false,
        ))
        .expect("snapshot");
    assert_eq!(snap.body.entities.len(), 3);
    // Ascending ids with live ids kept; kd enters with the next id.
    assert_eq!(snap.body.entities[0].entity, net(1));
    assert_eq!(snap.body.entities[1].entity, net(2));
    assert_eq!(snap.body.entities[2].entity, net(4));
    assert_eq!(snap.body.entities[0].health, Some(health(5, 10, false)));
    assert_eq!(snap.body.entities[1].health, None);
    assert_eq!(snap.body.entities[2].health, Some(health(2, 8, false)));
    assert_eq!(
        snap.body.turn,
        Some(TurnFact {
            active: None,
            round: 2
        })
    );
    // Dropped key retired.
    assert_eq!(v.net_id(kc), None);
    assert_eq!(v.entity(net(3)), None);
    assert_eq!(v.net_id(kd), Some(net(4)));
    assert_eq!(v.cursor().revision, 2);
    assert_eq!(v.cursor().tick, 11);
    // The next delta with the same facts re-emits nothing unchanged, so
    // `project_snapshot` must have updated `last_health` (and the entering
    // key's image) to exactly what the body says.
    let q = v
        .project_delta(&context(
            ep,
            12,
            1,
            &snap_facts,
            &empty,
            t_hidden,
            true,
            false,
        ))
        .expect("cutover delta");
    assert!(q.ops.is_empty());
    // Revision does not step for an empty page.
    assert_eq!(v.cursor().revision, 2);
    assert_eq!(v.cursor().tick, 11);
    assert_eq!(q.cursor, v.cursor());
    // A key dropped by a snapshot re-enters with a new id.
    let reenter = vec![
        fact(ka, true, true, Some(health(5, 10, false)), false),
        fact(kb, true, false, None, false),
        fact(kc, true, true, Some(health(1, 5, false)), false),
        fact(kd, true, true, Some(health(2, 8, false)), false),
    ];
    let r = v
        .project_delta(&context(ep, 13, 1, &reenter, &empty, t, true, false))
        .expect("re-enter dropped");
    assert!(r.ops.iter().any(|op| matches!(
        op,
        DeltaOp::EntityEnter { entity } if entity.get() == 5
    )));
    assert_eq!(v.net_id(kc), Some(net(5)));
    assert_eq!(v.cursor().revision, 3);
}

// ---------------------------------------------------------------------------
// I13 (items 15, 12). Bounds.
// ---------------------------------------------------------------------------

#[test]
fn i13_bounds_maximal_page_and_cycles() {
    let ep = epoch(0x1D);
    // 1,024 rows pass; 1,025 fail. Same for despawned.
    let (_arena, ids) = arena_n(MAX_VISIBLE_ENTITIES);
    let _ = _arena;
    let facts: Vec<EntityFact> = ids
        .iter()
        .map(|e| fact(key(1, *e), true, false, None, false))
        .collect();
    let empty: Vec<EntityKey> = vec![];
    let mut v = InterestView::new(ep);
    v.project_delta(&context(ep, 1, 1, &facts, &empty, None, false, false))
        .expect("1024 pass");
    assert_eq!(v.len(), MAX_VISIBLE_ENTITIES);
    let (_arena2, ids2) = arena_n(MAX_VISIBLE_ENTITIES + 1);
    let _ = _arena2;
    let too_many: Vec<EntityFact> = ids2
        .iter()
        .map(|e| fact(key(3, *e), true, false, None, false))
        .collect();
    let mut v_big = InterestView::new(ep);
    assert_eq!(
        v_big.project_delta(&context(ep, 1, 3, &too_many, &empty, None, false, false)),
        Err(InterestError::TooManyFacts)
    );
    assert_eq!(
        v_big.project_snapshot(&context(ep, 1, 3, &too_many, &empty, None, false, false)),
        Err(InterestError::TooManyFacts)
    );
    let despawned: Vec<EntityKey> = ids2.iter().map(|e| key(9, *e)).collect();
    assert_eq!(
        v_big.project_delta(&context(ep, 1, 3, &[], &despawned, None, false, false)),
        Err(InterestError::TooManyFacts)
    );
    assert_eq!(
        v_big.project_snapshot(&context(ep, 1, 3, &[], &despawned, None, false, false)),
        Err(InterestError::TooManyFacts)
    );
    // 1,024 despawned keys pass on both functions (nothing live, empty page).
    let (_arena3, ids3) = arena_n(MAX_VISIBLE_ENTITIES);
    let _ = _arena3;
    let despawned_ok: Vec<EntityKey> = ids3.iter().map(|e| key(9, *e)).collect();
    let mut v_ok = InterestView::new(ep);
    let pok = v_ok
        .project_delta(&context(ep, 1, 3, &[], &despawned_ok, None, false, false))
        .expect("1024 despawned pass delta");
    assert!(pok.ops.is_empty());
    let mut v_oks = InterestView::new(ep);
    let sok = v_oks
        .project_snapshot(&context(ep, 1, 3, &[], &despawned_ok, None, false, false))
        .expect("1024 despawned pass snapshot");
    assert!(sok.body.entities.is_empty());

    // Maximal page: 1,024 live replaced by 1,024 entering with health + turn.
    let (_arena_a, ids_a) = arena_n(MAX_VISIBLE_ENTITIES);
    let _ = _arena_a;
    let mut view = InterestView::new(ep);
    let first: Vec<EntityFact> = ids_a
        .iter()
        .map(|e| fact(key(1, *e), true, false, None, false))
        .collect();
    view.project_delta(&context(ep, 1, 1, &first, &empty, None, false, false))
        .expect("fill");
    let (_arena_b, ids_b) = arena_n(MAX_VISIBLE_ENTITIES);
    let _ = _arena_b;
    let second: Vec<EntityFact> = ids_b
        .iter()
        .map(|e| fact(key(2, *e), true, true, Some(health(1, 2, false)), false))
        .collect();
    let t = Some(turn_state(2, Some(ids_b[0]), 1));
    let page = view
        .project_delta(&context(ep, 2, 2, &second, &empty, t, true, false))
        .expect("maximal page");
    assert_eq!(page.ops.len(), MAX_PAGE_OPS);
    assert_eq!(MAX_PAGE_OPS, 3 * MAX_VISIBLE_ENTITIES + 1);
    // Chunks at MAX_DELTA_OPS all pass encode_delta.
    let mut start = 0;
    while start < page.ops.len() {
        let end = (start + MAX_DELTA_OPS).min(page.ops.len());
        let frame = DeltaFrame {
            lane: LANE_COMBAT,
            server_tick: 1,
            event_seq: 1,
            ops: page.ops[start..end].to_vec(),
        };
        encode_delta(&frame).expect("chunk encodes");
        start = end;
    }

    // 10,000 enter/leave cycles leave len() == 0 and ids strictly rising.
    let mut vc = InterestView::new(ep);
    let (_arena_c, ids_c) = arena_n(1);
    let _ = _arena_c;
    let k = key(5, ids_c[0]);
    let mut last_raw = 0u64;
    for tick in 0..10_000u64 {
        let f = vec![fact(k, true, false, None, false)];
        let p = vc
            .project_delta(&context(ep, tick, 5, &f, &empty, None, false, false))
            .expect("enter");
        match p.ops.as_slice() {
            [DeltaOp::EntityEnter { entity }] => {
                assert!(entity.get() > last_raw);
                last_raw = entity.get();
            }
            other => panic!("expected one enter, got {other:?}"),
        }
        let gone: Vec<EntityFact> = vec![];
        let q = vc
            .project_delta(&context(ep, tick, 5, &gone, &empty, None, false, false))
            .expect("leave");
        assert_eq!(q.ops.len(), 1);
    }
    assert_eq!(vc.len(), 0);

    // A 1,024-entity all-health body produced by `project_snapshot` passes
    // `encode_body` and `plan_transfer`.
    let (_arena_m, ids_m) = arena_n(MAX_VISIBLE_ENTITIES);
    let _ = _arena_m;
    let max_facts: Vec<EntityFact> = ids_m
        .iter()
        .map(|e| {
            fact(
                key(1, *e),
                true,
                true,
                Some(health(u32::MAX, u32::MAX, true)),
                false,
            )
        })
        .collect();
    let max_turn = Some(turn_state(1, Some(ids_m[0]), u64::MAX));
    let mut v_max = InterestView::new(ep);
    let max_snap = v_max
        .project_snapshot(&context(
            ep, 9, 1, &max_facts, &empty, max_turn, true, false,
        ))
        .expect("max snapshot");
    assert_eq!(max_snap.body.entities.len(), MAX_VISIBLE_ENTITIES);
    encode_body(&max_snap.body).expect("max body encodes");
    plan_transfer(
        [0x1D; 16],
        TransferId::new(1).expect("tid"),
        1,
        0,
        1,
        &max_snap.body,
    )
    .expect("max plans");
}

// ---------------------------------------------------------------------------
// I14 (items 1, 2). Errors on both functions.
// ---------------------------------------------------------------------------

#[test]
fn i14_error_precedence_atomicity_display() {
    let ep = epoch(0x1E);
    let bad_ep = epoch(0x1F);
    let (_arena, eid) = arena_one();
    let k = key(1, eid);
    let good = vec![fact(k, true, false, None, false)];
    let empty: Vec<EntityKey> = vec![];

    // Each of WrongEpoch, TooManyFacts, DuplicateFact on both functions,
    // view equals its clone, then a valid context gives the expected page.
    for variant in ["wrong_epoch", "too_many", "duplicate"] {
        let mut vd = InterestView::new(ep);
        let mut vs = InterestView::new(ep);
        let (facts, despawned): (Vec<EntityFact>, Vec<EntityKey>) = match variant {
            "wrong_epoch" => (good.clone(), vec![]),
            "too_many" => {
                let (_a, ids) = arena_n(MAX_VISIBLE_ENTITIES + 1);
                let _ = _a;
                (
                    ids.iter()
                        .map(|e| fact(key(1, *e), true, false, None, false))
                        .collect(),
                    vec![],
                )
            }
            _ => (
                vec![
                    fact(k, true, false, None, false),
                    fact(k, false, false, None, false),
                ],
                vec![],
            ),
        };
        let before_d = vd.clone();
        let before_s = vs.clone();
        let ctx_bad = match variant {
            "wrong_epoch" => context(bad_ep, 1, 1, &facts, &despawned, None, false, false),
            _ => context(ep, 1, 1, &facts, &despawned, None, false, false),
        };
        let want = match variant {
            "wrong_epoch" => InterestError::WrongEpoch,
            "too_many" => InterestError::TooManyFacts,
            _ => InterestError::DuplicateFact,
        };
        assert_eq!(vd.project_delta(&ctx_bad), Err(want));
        assert_eq!(vd, before_d);
        assert_eq!(
            vs.project_snapshot(&context(
                if variant == "wrong_epoch" { bad_ep } else { ep },
                1,
                1,
                &facts,
                &despawned,
                None,
                false,
                false
            )),
            Err(want)
        );
        assert_eq!(vs, before_s);
        // Valid context then gives the expected page.
        let p = vd
            .project_delta(&context(ep, 2, 1, &good, &empty, None, false, false))
            .expect("valid after error");
        assert_eq!(p.ops, vec![DeltaOp::EntityEnter { entity: net(1) }]);
    }

    // Combined faults report in item-1 order: epoch, length, duplicate.
    let (_arena_m, ids_m) = arena_n(MAX_VISIBLE_ENTITIES + 1);
    let _ = _arena_m;
    let mut many_dup: Vec<EntityFact> = ids_m
        .iter()
        .map(|e| fact(key(1, *e), true, false, None, false))
        .collect();
    many_dup[1] = many_dup[0].clone();
    let mut v = InterestView::new(ep);
    assert_eq!(
        v.project_delta(&context(
            bad_ep, 1, 1, &many_dup, &empty, None, false, false
        )),
        Err(InterestError::WrongEpoch)
    );
    assert_eq!(
        v.project_delta(&context(ep, 1, 1, &many_dup, &empty, None, false, false)),
        Err(InterestError::TooManyFacts)
    );
    let dup2 = vec![
        fact(k, true, false, None, false),
        fact(k, true, false, None, false),
    ];
    assert_eq!(
        v.project_delta(&context(ep, 1, 1, &dup2, &empty, None, false, false)),
        Err(InterestError::DuplicateFact)
    );

    // Repeat the rejections on a view with live mappings and a remembered
    // image, after a retirement and a mint, each compared with a pre-call
    // clone (contract item 2: `Err` changes nothing).
    {
        let (_arena_l, ids_l) = arena_n(3);
        let _ = _arena_l;
        let kla = key(1, ids_l[0]);
        let klb = key(1, ids_l[1]);
        let klc = key(1, ids_l[2]);
        let mut vl = InterestView::new(ep);
        let enter = vec![
            fact(kla, true, true, Some(health(4, 10, false)), false),
            fact(klb, true, true, Some(health(6, 10, false)), false),
        ];
        let t_live = Some(turn_state(1, Some(ids_l[0]), 1));
        vl.project_delta(&context(ep, 1, 1, &enter, &empty, t_live, true, false))
            .expect("live mappings with image");
        // Retirement: drop klb.
        let retire = vec![fact(kla, true, true, Some(health(4, 10, false)), false)];
        vl.project_delta(&context(ep, 2, 1, &retire, &empty, t_live, true, false))
            .expect("retire one");
        assert_eq!(vl.net_id(klb), None);
        // Mint: enter klc with a new id.
        let mint = vec![
            fact(kla, true, true, Some(health(4, 10, false)), false),
            fact(klc, true, true, Some(health(1, 2, false)), false),
        ];
        vl.project_delta(&context(ep, 3, 1, &mint, &empty, t_live, true, false))
            .expect("mint new");
        assert_eq!(vl.len(), 2);
        // Now provoke each rejection on both functions.
        let (_arena_t, ids_t) = arena_n(MAX_VISIBLE_ENTITIES + 1);
        let _ = _arena_t;
        let too_many: Vec<EntityFact> = ids_t
            .iter()
            .map(|e| fact(key(1, *e), true, false, None, false))
            .collect();
        let dup = vec![
            fact(kla, true, false, None, false),
            fact(kla, true, false, None, false),
        ];
        for (facts_bad, ds_bad, want) in [
            (mint.clone(), vec![], InterestError::WrongEpoch),
            (too_many.clone(), vec![], InterestError::TooManyFacts),
            (dup.clone(), vec![], InterestError::DuplicateFact),
        ] {
            let before = vl.clone();
            let epoch_use = if want == InterestError::WrongEpoch {
                bad_ep
            } else {
                ep
            };
            assert_eq!(
                vl.project_delta(&context(
                    epoch_use, 9, 1, &facts_bad, &ds_bad, None, false, false
                )),
                Err(want)
            );
            assert_eq!(vl, before, "delta {want:?} must not mutate");
            let before_s = vl.clone();
            assert_eq!(
                vl.project_snapshot(&context(
                    epoch_use, 9, 1, &facts_bad, &ds_bad, None, false, false
                )),
                Err(want)
            );
            assert_eq!(vl, before_s, "snapshot {want:?} must not mutate");
        }
        // A valid context then gives the expected page (unchanged: empty).
        let ok = vl
            .project_delta(&context(ep, 4, 1, &mint, &empty, t_live, true, false))
            .expect("valid after errors on live view");
        assert!(ok.ops.is_empty());
    }
    // `IdExhausted` precedence after `DuplicateFact` is not reachable from the
    // public API: exhaustion needs `next_id` near `u64::MAX`, which no bounded
    // call (at most 1,024 entering keys) can approach from `next_id == 1`.
    // Exhaustion is covered by the `#[cfg(test)] force_next_id` unit test
    // (I15 in `src/interest.rs`).

    // Display strings are exact.
    assert_eq!(
        format!("{}", InterestError::WrongEpoch),
        "WrongEpoch at projection/interest"
    );
    assert_eq!(
        format!("{}", InterestError::TooManyFacts),
        "TooManyFacts at projection/interest"
    );
    assert_eq!(
        format!("{}", InterestError::DuplicateFact),
        "DuplicateFact at projection/interest"
    );
    assert_eq!(
        format!("{}", InterestError::IdExhausted),
        "IdExhausted at projection/interest"
    );
}

// ---------------------------------------------------------------------------
// I16 (item 10). Hidden invariance over 50 scripted steps.
// ---------------------------------------------------------------------------

#[test]
fn i16_hidden_inputs_do_not_move_bytes() {
    let ep = epoch(0x20);
    let (_arena, ids) = arena_n(4);
    let _ = _arena;
    let va = ids[0];
    let vb = ids[1];
    let hidden = ids[2];
    let foreign_e = ids[3];
    let ka = key(1, va);
    let kb = key(1, vb);
    let kh = key(1, hidden);
    let kf = key(2, foreign_e);

    type StepRecord = (
        Vec<DeltaOp>,
        Vec<u8>,
        ViewCursor,
        usize,
        Option<NetId>,
        Option<NetId>,
    );
    let run = |with_extra: bool, active: EntityId| {
        let mut view = InterestView::new(ep);
        let mut out: Vec<StepRecord> = Vec::new();
        for step in 0..50u64 {
            let h = 10 + (step % 5) as u32;
            let mut facts = vec![
                fact(ka, true, true, Some(health(h, 20, false)), false),
                fact(kb, true, true, Some(health(3, 7, false)), false),
            ];
            if with_extra {
                // Hidden-only differences: an ungranted secret row and a
                // foreign row, present in one run and absent in the other.
                facts.push(fact(kh, false, false, Some(health(77, 99, false)), false));
                facts.push(fact(kf, true, true, Some(health(77, 50, false)), false));
            }
            // Hidden active differs between the runs but never renders: one
            // run names the hidden row, the other a foreign entity.
            let t = Some(turn_state(1, Some(active), step));
            // Foreign row kf (when present) is outside viewer area 1, so
            // suppressed; the hidden active renders `active: None` either way.
            let proj = view
                .project_delta(&context(ep, step, 1, &facts, &[], t, step % 3 == 0, false))
                .expect("step projects");
            let frame = DeltaFrame {
                lane: LANE_COMBAT,
                server_tick: step,
                event_seq: step + 1,
                ops: proj.ops.clone(),
            };
            out.push((
                proj.ops.clone(),
                encode_delta(&frame).expect("encodes"),
                proj.cursor,
                view.len(),
                view.net_id(ka),
                view.net_id(kb),
            ));
        }
        out
    };

    // One run carries the extra hidden row `kh` and the foreign row `kf` with
    // hidden active `hidden`; the other carries neither and uses a different
    // hidden active `foreign_e`. Ids, len, cursor and ops are identical.
    let first = run(true, hidden);
    let second = run(false, foreign_e);
    assert_eq!(first, second);

    // Control: the same difference on a visible, granted row changes bytes.
    let mut va_view = InterestView::new(ep);
    let mut vb_view = InterestView::new(ep);
    let fa = vec![fact(ka, true, true, Some(health(77, 20, false)), false)];
    let fb = vec![fact(ka, true, true, Some(health(191, 20, false)), false)];
    let pa = va_view
        .project_delta(&context(ep, 0, 1, &fa, &[], None, false, false))
        .expect("a");
    let pb = vb_view
        .project_delta(&context(ep, 0, 1, &fb, &[], None, false, false))
        .expect("b");
    let ba = encode_delta(&DeltaFrame {
        lane: LANE_COMBAT,
        server_tick: 0,
        event_seq: 1,
        ops: pa.ops,
    })
    .expect("encodes");
    let bb = encode_delta(&DeltaFrame {
        lane: LANE_COMBAT,
        server_tick: 0,
        event_seq: 1,
        ops: pb.ops,
    })
    .expect("encodes");
    assert_ne!(ba, bb);
}

// ---------------------------------------------------------------------------
// I17 (items 5, 13). Convergence over a 2,000-step seeded schedule.
// ---------------------------------------------------------------------------

#[test]
fn i17_seeded_convergence_two_viewers() {
    let ep = epoch(0x21);
    let mut arena = GenerationalArena::new();
    let pool: Vec<EntityId> = (0..8).map(|_| arena.insert(())).collect();
    let areas = [1u128, 2u128];
    let mut rng = DeterministicRng::from_seed(27);
    let stream = rng.stream("interest");

    let mut va = InterestView::new(ep);
    let mut vb = InterestView::new(ep);
    let mut ma = Mirror::new();
    let mut mb = Mirror::new();
    let mut oa = Oracle::new();
    let mut ob = Oracle::new();
    // Ground-truth grant/health store keyed by EntityKey: (present,
    // health grant, value). The value persists while the grant is cleared so
    // the schedule exercises clear then re-grant with the same value.
    let mut store: BTreeMap<EntityKey, (bool, bool, Option<HealthFact>)> = BTreeMap::new();
    for e in &pool {
        for a in areas {
            store.insert(
                EntityKey {
                    area: area(a),
                    entity: *e,
                },
                (false, false, None),
            );
        }
    }
    let mut turn_a: Option<TurnState> = Some(TurnState {
        area: area(1),
        active: Some(pool[0]),
        round: 0,
    });
    let mut turn_b: Option<TurnState> = Some(TurnState {
        area: area(2),
        active: Some(pool[1]),
        round: 0,
    });
    let mut grant_a = true;
    let mut grant_b = true;

    for step in 0..2_000u64 {
        let pick = stream.next_u32() % 8;
        match pick {
            0 | 1 => {
                // Enter/grant flip on a random key, including clear and
                // re-grant of the health grant with the value kept.
                let e = pool[(stream.next_u32() as usize) % pool.len()];
                let a = areas[(stream.next_u32() as usize) % 2];
                let k = EntityKey {
                    area: area(a),
                    entity: e,
                };
                let sub = stream.next_u32() % 4;
                match sub {
                    0 => {
                        let hv = Some(health(
                            stream.next_u32() % 11,
                            10,
                            stream.next_u32().is_multiple_of(7),
                        ));
                        store.insert(k, (true, true, hv));
                    }
                    1 => {
                        // Present but health grant off; the secret value is
                        // kept so a later re-grant emits the same value.
                        let hv = Some(health(
                            stream.next_u32() % 11,
                            10,
                            stream.next_u32().is_multiple_of(7),
                        ));
                        store.insert(k, (true, false, hv));
                    }
                    2 => {
                        // Clear the health grant, keeping the value.
                        if let Some((present, _, hv)) = store.get(&k).cloned() {
                            if present {
                                store.insert(k, (true, false, hv));
                            } else {
                                store.insert(k, (false, false, hv));
                            }
                        }
                    }
                    _ => {
                        // Re-grant with the same value (or a fresh one if none).
                        if let Some((_, _, hv)) = store.get(&k).cloned() {
                            let hv2 = hv.or(Some(health(
                                stream.next_u32() % 11,
                                10,
                                stream.next_u32().is_multiple_of(7),
                            )));
                            store.insert(k, (true, true, hv2));
                        }
                    }
                }
            }
            2 => {
                // Area move: copy presence to the sibling area key.
                let e = pool[(stream.next_u32() as usize) % pool.len()];
                let from = areas[(stream.next_u32() as usize) % 2];
                let to = if from == 1 { 2 } else { 1 };
                let kf = EntityKey {
                    area: area(from),
                    entity: e,
                };
                let kt = EntityKey {
                    area: area(to),
                    entity: e,
                };
                let val = store.get(&kf).cloned().unwrap_or((false, false, None));
                store.insert(kf, (false, false, val.2.clone()));
                store.insert(kt, val);
            }
            3 => {
                // Turn update for a random viewer, including grant clears and
                // moves to the other area so the turn stops rendering and the
                // last disclosed value must persist.
                let pick_viewer = stream.next_u32() % 2;
                let e = pool[(stream.next_u32() as usize) % pool.len()];
                let round = (stream.next_u32() % 20) as u64;
                let mode = stream.next_u32() % 4;
                if pick_viewer == 0 {
                    match mode {
                        0 => {
                            turn_a = Some(TurnState {
                                area: area(1),
                                active: Some(e),
                                round,
                            });
                            grant_a = true;
                        }
                        1 => {
                            // Clear the turn grant; the rendered turn is None.
                            grant_a = false;
                        }
                        2 => {
                            // Move the turn to the other area; viewer A no
                            // longer renders it.
                            turn_a = Some(TurnState {
                                area: area(2),
                                active: Some(e),
                                round,
                            });
                            grant_a = true;
                        }
                        _ => {
                            // Re-grant the same turn in the matching area.
                            if let Some(t) = turn_a {
                                turn_a = Some(TurnState {
                                    area: area(1),
                                    active: t.active,
                                    round: t.round,
                                });
                            } else {
                                turn_a = Some(TurnState {
                                    area: area(1),
                                    active: Some(e),
                                    round,
                                });
                            }
                            grant_a = true;
                        }
                    }
                } else {
                    match mode {
                        0 => {
                            turn_b = Some(TurnState {
                                area: area(2),
                                active: Some(e),
                                round,
                            });
                            grant_b = true;
                        }
                        1 => {
                            grant_b = false;
                        }
                        2 => {
                            turn_b = Some(TurnState {
                                area: area(1),
                                active: Some(e),
                                round,
                            });
                            grant_b = true;
                        }
                        _ => {
                            if let Some(t) = turn_b {
                                turn_b = Some(TurnState {
                                    area: area(2),
                                    active: t.active,
                                    round: t.round,
                                });
                            } else {
                                turn_b = Some(TurnState {
                                    area: area(2),
                                    active: Some(e),
                                    round,
                                });
                            }
                            grant_b = true;
                        }
                    }
                }
            }
            4 => {
                // Snapshot one viewer.
                for (viewer, mirror, oracle, interest, turn, grant) in [
                    (&mut va, &mut ma, &mut oa, 1u128, turn_a, grant_a),
                    (&mut vb, &mut mb, &mut ob, 2u128, turn_b, grant_b),
                ] {
                    if stream.next_u32().is_multiple_of(2) {
                        continue;
                    }
                    let facts: Vec<EntityFact> = store
                        .iter()
                        .map(|(k, (present, hgrant, hv))| EntityFact {
                            key: *k,
                            fields: FieldMask {
                                presence: *present,
                                health: *hgrant,
                            },
                            health: hv.clone(),
                            resend_health: false,
                        })
                        .collect();
                    let ctx = context(ep, step, interest, &facts, &[], turn, grant, false);
                    let snap = viewer.project_snapshot(&ctx).expect("snapshot");
                    mirror
                        .apply_body(&snap.body)
                        .expect("mirror applies snapshot");
                    oracle
                        .step_snapshot(&facts, area(interest), turn, grant)
                        .expect("oracle steps");
                    assert_view_matches_oracle(viewer, oracle);
                    assert_mirror_matches_oracle(mirror, oracle);
                }
                continue;
            }
            _ => {
                // Health value change on a random live-ish key.
                let e = pool[(stream.next_u32() as usize) % pool.len()];
                let a = areas[(stream.next_u32() as usize) % 2];
                let k = EntityKey {
                    area: area(a),
                    entity: e,
                };
                if let Some((present, hgrant, _)) = store.get(&k).cloned() {
                    if present {
                        store.insert(
                            k,
                            (
                                true,
                                hgrant,
                                Some(health(
                                    stream.next_u32() % 11,
                                    10,
                                    stream.next_u32().is_multiple_of(9),
                                )),
                            ),
                        );
                    }
                }
            }
        }
        // Delta both viewers every step.
        for (viewer, mirror, oracle, interest, turn, grant) in [
            (&mut va, &mut ma, &mut oa, 1u128, turn_a, grant_a),
            (&mut vb, &mut mb, &mut ob, 2u128, turn_b, grant_b),
        ] {
            let facts: Vec<EntityFact> = store
                .iter()
                .map(|(k, (present, hgrant, hv))| EntityFact {
                    key: *k,
                    fields: FieldMask {
                        presence: *present,
                        health: *hgrant,
                    },
                    health: hv.clone(),
                    resend_health: false,
                })
                .collect();
            let ctx = context(ep, step, interest, &facts, &[], turn, grant, false);
            let proj = viewer.project_delta(&ctx).expect("delta steps");
            mirror.apply(&proj).expect("mirror applies delta");
            oracle
                .step_delta(&facts, area(interest), turn, grant)
                .expect("oracle steps delta");
            assert_view_matches_oracle(viewer, oracle);
            assert_mirror_matches_oracle(mirror, oracle);
        }
    }
}

// ---------------------------------------------------------------------------
// I18 (corrupt and drop). Every corruption fails loudly.
// ---------------------------------------------------------------------------

#[test]
fn i18_corruptions_fail_loudly() {
    let ep = epoch(0x22);
    let (_arena, ids) = arena_n(2);
    let a = ids[0];
    let b = ids[1];
    let _ = _arena;
    let ka = key(1, a);
    let kb = key(1, b);
    let empty: Vec<EntityKey> = vec![];
    let facts = vec![
        fact(ka, true, true, Some(health(4, 10, false)), false),
        fact(kb, true, true, Some(health(5, 10, false)), false),
    ];
    let mut view = InterestView::new(ep);
    let page = view
        .project_delta(&context(ep, 1, 1, &facts, &empty, None, false, false))
        .expect("base page");
    assert!(page.enters == 2);

    // Drop an EntityEnter: Health names an unknown id.
    {
        let mut mirror = Mirror::new();
        let mut dropped = page.clone();
        dropped
            .ops
            .retain(|op| !matches!(op, DeltaOp::EntityEnter { .. }));
        dropped.enters = 0;
        assert!(mirror.apply(&dropped).is_err());
    }
    // Move Health before its enter.
    {
        let mut mirror = Mirror::new();
        let mut reordered = page.clone();
        if reordered.ops.len() >= 3 {
            reordered.ops.swap(0, 2);
        }
        assert!(mirror.apply(&reordered).is_err());
    }
    // Alter a health value: converges structurally but mismatches the oracle.
    // Control: the untouched page agrees with the oracle.
    {
        let mut mirror = Mirror::new();
        mirror.apply(&page).expect("base applies");
        let mut oracle = Oracle::new();
        oracle
            .step_delta(&facts, area(1), None, false)
            .expect("oracle steps");
        assert_mirror_matches_oracle(&mirror, &oracle);
        // Corrupt one value in the mirror; the comparator must fail loudly.
        if let Some(slot) = mirror.live.get_mut(&1) {
            *slot = Some(health(99, 10, false));
        }
        let mismatch = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert_mirror_matches_oracle(&mirror, &oracle);
        }));
        assert!(mismatch.is_err(), "altered health must fail the oracle");
    }
    // Drop a page: the next page names unknown ids.
    {
        let mut view2 = InterestView::new(ep);
        let p1 = view2
            .project_delta(&context(ep, 1, 1, &facts, &empty, None, false, false))
            .expect("p1");
        let facts2 = vec![
            fact(ka, true, true, Some(health(8, 10, false)), false),
            fact(kb, true, true, Some(health(5, 10, false)), false),
        ];
        let p2 = view2
            .project_delta(&context(ep, 2, 1, &facts2, &empty, None, false, false))
            .expect("p2");
        let mut mirror = Mirror::new();
        // Skip p1, apply p2 alone: Health for never-entered ids fails.
        assert!(mirror.apply(&p2).is_err());
        // Control: untouched pages converge.
        let mut good = Mirror::new();
        good.apply(&p1).expect("p1 applies");
        good.apply(&p2).expect("p2 applies");
    }
    // Duplicate a removal.
    {
        let mut view3 = InterestView::new(ep);
        view3
            .project_delta(&context(ep, 1, 1, &facts, &empty, None, false, false))
            .expect("enter");
        let gone: Vec<EntityFact> = vec![];
        let leave = view3
            .project_delta(&context(ep, 2, 1, &gone, &empty, None, false, false))
            .expect("leave page");
        assert_eq!(leave.ops.len(), 2);
        let mut mirror = Mirror::new();
        let enter_page = StateProjection {
            ops: vec![
                DeltaOp::EntityEnter { entity: net(1) },
                DeltaOp::EntityEnter { entity: net(2) },
                DeltaOp::Health {
                    entity: net(1),
                    health: 4,
                    max_health: 10,
                    dead: false,
                },
                DeltaOp::Health {
                    entity: net(2),
                    health: 5,
                    max_health: 10,
                    dead: false,
                },
            ],
            enters: 2,
            cursor: ViewCursor::default(),
        };
        mirror.apply(&enter_page).expect("enters apply");
        mirror.apply(&leave).expect("first removal applies");
        assert!(mirror.apply(&leave).is_err());
    }
}

// ---------------------------------------------------------------------------
// I19 (wire impact). Fixed page through both codecs.
// ---------------------------------------------------------------------------

#[test]
fn i19_fixed_page_wire_parity() {
    let ep = epoch(0x23);
    let (_arena, ids) = arena_n(1);
    let _ = _arena;
    let k = key(1, ids[0]);
    let empty: Vec<EntityKey> = vec![];
    let mut view = InterestView::new(ep);
    let facts = vec![fact(k, true, true, Some(health(3, 12, false)), false)];
    let t = Some(turn_state(1, Some(ids[0]), 2));
    let page = view
        .project_delta(&context(ep, 9, 1, &facts, &empty, t, true, false))
        .expect("fixed page");
    assert_eq!(page.enters, 1);
    let frame = DeltaFrame {
        lane: LANE_COMBAT,
        server_tick: 100,
        event_seq: 55,
        ops: page.ops.clone(),
    };
    let v1 = encode_delta(&frame).expect("v1 encodes");
    // Literal wire bytes for this exact page (version 1, lane 0, tick 100,
    // seq 55, 3 ops: enter 1, health 1:3/12, turn 1:2).
    let hand = {
        let mut out = vec![PROTOCOL_VERSION, LANE_COMBAT];
        out.extend(leb(100));
        out.extend(leb(55));
        out.extend(leb(3));
        out.push(DELTA_TAG_ENTITY_ENTER);
        out.extend(leb(1));
        out.push(DELTA_TAG_HEALTH);
        out.extend(leb(1));
        out.extend(leb(3));
        out.extend(leb(12));
        out.push(0u8); // dead = false
        out.push(DELTA_TAG_TURN);
        out.push(1u8); // Some(active)
        out.extend(leb(1));
        out.extend(leb(2));
        out
    };
    // Tag constants pin the layout; the byte vector above is the literal.
    assert_eq!(v1, hand);
    assert_eq!(decode_delta(&v1).expect("decodes"), frame);
    let legacy_ops: Vec<DeltaOpV2> = page.ops.into_iter().map(DeltaOpV2::Legacy).collect();
    let frame_v2 = DeltaFrameV2 {
        lane: LANE_COMBAT,
        server_tick: 100,
        event_seq: 55,
        ops: legacy_ops,
    };
    let v2 = codec_v2::encode_delta(&frame_v2).expect("v2 encodes");
    assert_eq!(v2[0], 2);
    assert_eq!(v1[0], 1);
    assert_eq!(&v2[1..], &v1[1..]);
}

fn leb(mut value: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let low = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(low);
            break;
        }
        out.push(low | 0x80);
    }
    out
}
