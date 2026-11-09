//! Authoritative movement state, routes and operations (T036a, ADR-0036).
//!
//! Positions are `[Fx16_16; 3]` as `[x, y, z]`, `y` up, metres (U1). A
//! [`Mover`] holds one creature's position, per-tick speed, turn-based step
//! length and optional [`ActiveRoute`]; the movers live in
//! [`World::movers`](crate::World::movers) and the one bound navmesh identity in
//! [`World::nav`](crate::World::nav). The [`Transform`] store is only the movers'
//! `f32` mirror, written here and read by no rule (U2, U6).
//!
//! Sim never imports the navigation crate (A2): the host bakes and queries a
//! mesh, then hands sim a whole [`MoveRoute`] it validated nothing about except
//! what [`validate_route`] can check without geometry. The arithmetic below is
//! integer-only and total: no input panics.
//!
//! This module holds the pure arithmetic, the data types, mover creation, the
//! four operations [`World::bind_nav`], [`World::place`], [`World::set_route`]
//! and [`World::clear_route`] (T036a), and the stage-4 walk with its
//! [`MovementFact`]s and the crate-private `stop` the turn-based transitions
//! use (T036b); then the turn-based move: [`TurnMovement`], [`route_cost`],
//! [`validate_move`] and [`perform_move`] (T036c).

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crpg_core::{EntityId, Fx16_16, GenerationalArena, Interners, Ulid};
use crpg_data::{
    ExprWire, LengthWire, Placement as AuthoredPlacement, Ruleset as AuthoredRuleset, StatKindWire,
    StatValueWire, MAX_SPEED_PER_TICK_RAW,
};
use crpg_rules::{
    Expr, ModifierPipeline, QueryContext, StatBlock, StatDefinition, StatKind, StatValue, TagSet,
};

use crate::combat::{
    pool_balance, spend_pool, validate_action, CombatAction, CombatError, EncounterMode,
};
use crate::transform::Transform;
use crate::world::World;

/// Largest absolute raw coordinate of any position or route point (2^29, ADR-0036 U3).
pub const COORD_LIMIT_RAW: i32 = 1 << 29;
/// Most points in a route, start included (ADR-0036 U7).
pub const MAX_ROUTE_POINTS: usize = 64;
/// Longest route, raw U4 length (2^28, ADR-0036 U7).
pub const MAX_ROUTE_LENGTH_RAW: u64 = 1 << 28;

/// Content identity of a navmesh: BLAKE3 of its canonical bytes (ADR-0036 R2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct NavRevision(pub [u8; 32]);

impl NavRevision {
    /// `blake3::hash(bytes)`.
    pub fn of_bytes(bytes: &[u8]) -> Self {
        Self(*blake3::hash(bytes).as_bytes())
    }
}

/// The host's identity for one route request; stored and echoed, never interpreted (R3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RouteTag(pub u64);

/// A whole route the host supplies (R1); replaced whole, never edited (R5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MoveRoute {
    /// The navmesh the route was computed on.
    pub revision: NavRevision,
    /// The host's request identity.
    pub tag: RouteTag,
    /// The polyline, start first.
    pub points: Vec<[Fx16_16; 3]>,
}

/// How rule-visible distance is measured (ADR-0036 S2); closed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Measure {
    /// The U4 route length.
    #[default]
    Euclidean,
}

/// The turn-based movement rules one encounter runs under (ADR-0036 C1, S2).
///
/// Persisted inside the combat definition's non-legacy form, only when `Some`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurnMovement {
    /// How a move's length is measured.
    pub measure: Measure,
    /// The pool template moves are paid from.
    pub pool: Ulid,
    /// Pool units per step, at least one.
    pub step_cost: u32,
}

/// One creature's authoritative movement state (fields private).
///
/// The position, speed and step are fixed-point; the route, when present, is
/// the one being walked. Built at registration or encounter start from the
/// ruleset's movement rules; changed only by the operations of this module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mover {
    position: [Fx16_16; 3],
    speed: Fx16_16,
    step: Fx16_16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    route: Option<ActiveRoute>,
}

impl Mover {
    /// Where the creature stands.
    pub fn position(&self) -> [Fx16_16; 3] {
        self.position
    }

    /// Walking distance per tick, raw `1..=MAX_SPEED_PER_TICK_RAW`.
    pub fn speed(&self) -> Fx16_16 {
        self.speed
    }

    /// The turn-based step length, raw at least one (stored at creation).
    pub fn step(&self) -> Fx16_16 {
        self.step
    }

    /// The route being walked, if any.
    pub fn route(&self) -> Option<&ActiveRoute> {
        self.route.as_ref()
    }

    /// This mover without its route, for the area transfer (M6).
    pub(crate) fn without_route(&self) -> Self {
        Self {
            position: self.position,
            speed: self.speed,
            step: self.step,
            route: None,
        }
    }
}

/// Why stage 4 dropped a route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelReason {
    /// The mover is dead (Reading 6).
    Dead,
}

/// One stage-4 fact, for the host; never a history event (ADR-0036 W1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MovementFact {
    /// The mover reached the last point of its route; the route is cleared.
    Arrived {
        /// The mover's entity.
        entity: EntityId,
        /// The finished route's tag.
        tag: RouteTag,
    },
    /// Stage 4 dropped the route without finishing it.
    Cancelled {
        /// The mover's entity.
        entity: EntityId,
        /// The dropped route's tag.
        tag: RouteTag,
        /// Why the route was dropped.
        reason: CancelReason,
    },
}

/// A route being walked (fields private).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveRoute {
    route: MoveRoute,
    segment: u16,
    along: u64,
}

impl ActiveRoute {
    /// The route as the host supplied it.
    pub fn route(&self) -> &MoveRoute {
        &self.route
    }

    /// The host's request identity.
    pub fn tag(&self) -> RouteTag {
        self.route.tag
    }

    /// The index of the segment being walked.
    pub fn segment(&self) -> u16 {
        self.segment
    }

    /// Raw U4 distance walked into that segment.
    pub fn along(&self) -> u64 {
        self.along
    }
}

/// Every movement failure (ADR-0036 M5); `Display` is `<Variant> at <location>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoveError {
    /// The entity is not live.
    AbsentEntity {
        /// The absent entity.
        entity: EntityId,
    },
    /// The live entity holds no mover.
    NotMover {
        /// The entity.
        entity: EntityId,
    },
    /// The mover's creature is dead.
    DeadCombatant {
        /// The dead entity.
        entity: EntityId,
    },
    /// The entity is a combatant of a turn-based encounter.
    TurnBased {
        /// The entity.
        entity: EntityId,
    },
    /// A route has fewer than 2 or more than [`MAX_ROUTE_POINTS`] points.
    RouteSize {
        /// The number of points supplied.
        points: usize,
    },
    /// A route point lies outside the coordinate limit.
    OutOfBounds {
        /// The first offending point.
        index: usize,
    },
    /// Two consecutive route points have the same horizontal position.
    ZeroSegment {
        /// The first segment of zero length.
        index: usize,
    },
    /// The route is longer than [`MAX_ROUTE_LENGTH_RAW`].
    TooLong {
        /// The route's raw U4 length.
        length: u64,
    },
    /// No navmesh revision is bound.
    NavUnbound,
    /// The route was computed on another revision than the bound one.
    StaleRevision,
    /// The route's first point is not the mover's position.
    StartMismatch,
    /// The turn-based move costs more than the pool holds.
    CannotAfford {
        /// The move's cost in pool units.
        cost: u64,
        /// The pool's current balance.
        current: u32,
    },
    /// A turn legality check failed.
    Combat(CombatError),
}

impl fmt::Display for MoveError {
    /// Renders `<Variant> at <location>`; `Combat` renders its inner error.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AbsentEntity { entity } => write!(f, "AbsentEntity at movers/{entity:?}"),
            Self::NotMover { entity } => write!(f, "NotMover at movers/{entity:?}"),
            Self::DeadCombatant { entity } => write!(f, "DeadCombatant at movers/{entity:?}"),
            Self::TurnBased { entity } => write!(f, "TurnBased at movers/{entity:?}"),
            Self::RouteSize { .. } => write!(f, "RouteSize at route/points"),
            Self::OutOfBounds { .. } => write!(f, "OutOfBounds at route/points"),
            Self::ZeroSegment { .. } => write!(f, "ZeroSegment at route/points"),
            Self::TooLong { .. } => write!(f, "TooLong at route/length"),
            Self::NavUnbound => write!(f, "NavUnbound at nav"),
            Self::StaleRevision => write!(f, "StaleRevision at route/revision"),
            Self::StartMismatch => write!(f, "StartMismatch at route/points"),
            Self::CannotAfford { .. } => write!(f, "CannotAfford at combat/pool"),
            Self::Combat(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for MoveError {
    /// The wrapped combat error for `Combat`, else `None`.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Combat(error) => Some(error),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Arithmetic (U4-U6, S2, S3). Integer-only and total.
// ---------------------------------------------------------------------------

/// `ceil(sqrt(n))`: `s + (s * s < n) as u128` with `s = n.isqrt()`.
fn ceil_sqrt(n: u128) -> u128 {
    let s = n.isqrt();
    s + u128::from(s * s < n)
}

/// U4: horizontal length, `ceil_sqrt(dx^2 + dz^2)` over raw values (B-U1).
///
/// `y` is ignored. The squares are taken in `u128`, so no input overflows, and
/// the result always fits `u64` (the radicand is below 2^66).
pub fn seg_len(a: [Fx16_16; 3], b: [Fx16_16; 3]) -> u64 {
    let dx = u128::from((i64::from(b[0].to_raw()) - i64::from(a[0].to_raw())).unsigned_abs());
    let dz = u128::from((i64::from(b[2].to_raw()) - i64::from(a[2].to_raw())).unsigned_abs());
    // `ceil_sqrt` of a value below 2^66 is below 2^33, so the cast is lossless.
    ceil_sqrt(dx * dx + dz * dz) as u64
}

/// U5: the point `d` along a segment of length `len` (B-U2).
///
/// `len == 0` gives `a`; `d >= len` gives `b` exactly; otherwise each component
/// is `a + ((b - a) * d) / len` in `i128` with truncating division. It lies
/// between `a` and `b`, so it fits `i32`.
pub fn point_at(a: [Fx16_16; 3], b: [Fx16_16; 3], len: u64, d: u64) -> [Fx16_16; 3] {
    if len == 0 {
        return a;
    }
    if d >= len {
        return b;
    }
    let component = |index: usize| {
        let from = i128::from(a[index].to_raw());
        let to = i128::from(b[index].to_raw());
        let value = from + (to - from) * i128::from(d) / i128::from(len);
        let raw = i32::try_from(value).unwrap_or(if value < 0 { i32::MIN } else { i32::MAX });
        Fx16_16::from_raw(raw)
    };
    [component(0), component(1), component(2)]
}

/// Whether a coordinate lies within [`COORD_LIMIT_RAW`] (never `abs()`, which
/// overflows on `i32::MIN`).
fn inside(v: Fx16_16) -> bool {
    v.to_raw().unsigned_abs() <= COORD_LIMIT_RAW as u32
}

/// Whether all three components of a point lie inside the limit.
pub(crate) fn point_inside(point: [Fx16_16; 3]) -> bool {
    point.iter().all(|value| inside(*value))
}

/// C4/U4: the sum of segment lengths (B-U3).
///
/// `None` for fewer than two points, any component outside the coordinate
/// limit, or a `u64` overflow.
pub fn route_length(points: &[[Fx16_16; 3]]) -> Option<u64> {
    if points.len() < 2 || !points.iter().all(|point| point_inside(*point)) {
        return None;
    }
    points.windows(2).try_fold(0_u64, |sum, pair| {
        sum.checked_add(seg_len(pair[0], pair[1]))
    })
}

/// S2: the rule-visible length of a route (B-U3).
pub fn route_measure(measure: Measure, points: &[[Fx16_16; 3]]) -> Option<u64> {
    match measure {
        Measure::Euclidean => route_length(points),
    }
}

/// S3: a destination snapped by `measure`; the identity in v1 (B-U4).
pub fn snap(measure: Measure, point: [Fx16_16; 3]) -> [Fx16_16; 3] {
    match measure {
        Measure::Euclidean => point,
    }
}

/// U6: `(raw(v) as f32) / 65536.0`, for the mirror and presentation only (B-U5).
pub fn to_f32(v: Fx16_16) -> f32 {
    (v.to_raw() as f32) / 65536.0
}

/// R4's geometry checks, in order: size, bounds, zero segments, total length.
/// Returns the route's U4 length.
fn validate_geometry(points: &[[Fx16_16; 3]]) -> Result<u64, MoveError> {
    if !(2..=MAX_ROUTE_POINTS).contains(&points.len()) {
        return Err(MoveError::RouteSize {
            points: points.len(),
        });
    }
    if let Some(index) = points.iter().position(|point| !point_inside(*point)) {
        return Err(MoveError::OutOfBounds { index });
    }
    if let Some(index) = points
        .windows(2)
        .position(|pair| seg_len(pair[0], pair[1]) == 0)
    {
        return Err(MoveError::ZeroSegment { index });
    }
    let total = route_length(points).unwrap_or(u64::MAX);
    if total > MAX_ROUTE_LENGTH_RAW {
        return Err(MoveError::TooLong { length: total });
    }
    Ok(total)
}

/// R4: validates a route against the bound revision and the mover's position;
/// returns the route's U4 length (B-R1).
///
/// Checks in order, the first failure winning: size, bounds, zero segments,
/// total length, binding, revision, start.
pub fn validate_route(
    route: &MoveRoute,
    nav: Option<NavRevision>,
    start: [Fx16_16; 3],
) -> Result<u64, MoveError> {
    let total = validate_geometry(&route.points)?;
    let Some(bound) = nav else {
        return Err(MoveError::NavUnbound);
    };
    if bound != route.revision {
        return Err(MoveError::StaleRevision);
    }
    if route.points[0] != start {
        return Err(MoveError::StartMismatch);
    }
    Ok(total)
}

// ---------------------------------------------------------------------------
// Mirror and load coherence
// ---------------------------------------------------------------------------

/// The mirror of `mover` with `velocity` (M3).
pub(crate) fn mirror(mover: &Mover, velocity: [f32; 3]) -> Transform {
    Transform {
        position: [
            to_f32(mover.position[0]),
            to_f32(mover.position[1]),
            to_f32(mover.position[2]),
        ],
        velocity,
    }
}

/// Writes `mirror(mover, zero)` under `entity`, replacing any transform.
pub(crate) fn write_mirror(world: &mut World, entity: EntityId, mover: &Mover) {
    world
        .transforms_mut()
        .insert(entity, mirror(mover, [0.0; 3]));
}

/// The first load rule L2-L12 that one mover breaks, as its text; `None` when
/// the mover is coherent. L1 (a non-live entity) is the caller's, which holds
/// the arena. `transform` is the entity's transform entry, `nav` the bound
/// revision (B-L6).
pub(crate) fn load_failure(
    mover: &Mover,
    transform: Option<&Transform>,
    nav: Option<NavRevision>,
) -> Option<&'static str> {
    if !(1..=MAX_SPEED_PER_TICK_RAW).contains(&mover.speed.to_raw()) {
        return Some("mover speed outside its limits");
    }
    if mover.step.to_raw() < 1 {
        return Some("mover step is not positive");
    }
    if !point_inside(mover.position) {
        return Some("mover position outside the coordinate limit");
    }
    let Some(transform) = transform else {
        return Some("mover without a transform");
    };
    let agrees = (0..3).all(|axis| transform.position[axis] == to_f32(mover.position[axis]));
    if !agrees {
        return Some("mover transform disagrees with its position");
    }
    let Some(active) = &mover.route else {
        return None;
    };
    let Some(bound) = nav else {
        return Some("mover route without a bound navmesh");
    };
    if bound != active.route.revision {
        return Some("mover route revision differs from the bound navmesh");
    }
    let points = &active.route.points;
    if validate_geometry(points).is_err() {
        return Some("mover route fails validation");
    }
    let segment = usize::from(active.segment);
    if segment >= points.len() - 1 {
        return Some("mover route segment out of range");
    }
    let len = seg_len(points[segment], points[segment + 1]);
    if active.along >= len {
        return Some("mover route progress out of range");
    }
    if mover.position != point_at(points[segment], points[segment + 1], len, active.along) {
        return Some("mover position disagrees with its route");
    }
    None
}

// ---------------------------------------------------------------------------
// Creation (M2, Readings 2-5)
// ---------------------------------------------------------------------------

/// Evaluates one derived `Fixed` stat by the rules kernel over `fixed`, or
/// `None` when the resolution fails (B-L2). Nothing is interned into a world.
fn evaluate_derived_fixed(
    ruleset: &AuthoredRuleset,
    fixed: &[(String, Fx16_16)],
    name: &str,
) -> Option<Fx16_16> {
    let fixed_names: Vec<&str> = ruleset
        .stats
        .iter()
        .filter(|declaration| matches!(declaration.kind, StatKindWire::Fixed))
        .map(|declaration| declaration.name.as_str())
        .collect();
    let mut scratch = Interners::new();
    // Every handle first, in authored order, so forward references resolve.
    let ids: Vec<_> = fixed_names
        .iter()
        .map(|stat| scratch.intern_stat(stat))
        .collect();
    let mut definitions = Vec::with_capacity(ids.len());
    for (declaration_name, id) in fixed_names.iter().zip(&ids) {
        let declaration = ruleset
            .stats
            .iter()
            .find(|declaration| declaration.name == *declaration_name)?;
        let derived = match &declaration.derived {
            Some(expr) => Some(fixed_expr(expr, &fixed_names, &mut scratch)?),
            None => None,
        };
        definitions.push(StatDefinition {
            id: *id,
            kind: StatKind::Fixed,
            derived,
        });
    }
    let mut block = StatBlock::new();
    for (stat, value) in fixed {
        let id = scratch.stat(stat)?;
        block.insert(id, StatValue::Fixed(*value)).ok()?;
    }
    let pipeline = ModifierPipeline::new(definitions, BTreeMap::new()).ok()?;
    let mut probe_arena: GenerationalArena<()> = GenerationalArena::new();
    let probe = probe_arena.insert(());
    let no_tags = TagSet::new();
    let context = QueryContext {
        entity: probe,
        stats: &block,
        tags: &no_tags,
        modifiers: &[],
    };
    let stat = scratch.stat(name)?;
    match pipeline.query(probe, stat, &context) {
        Ok((StatValue::Fixed(value), _)) => Some(value),
        _ => None,
    }
}

/// Converts one authored formula node for node into a kernel expression over
/// `Fixed` stats; a non-`Fixed` literal or name fails the conversion.
fn fixed_expr(expr: &ExprWire, fixed_names: &[&str], scratch: &mut Interners) -> Option<Expr> {
    fn pair(
        left: &ExprWire,
        right: &ExprWire,
        fixed_names: &[&str],
        scratch: &mut Interners,
    ) -> Option<(Box<Expr>, Box<Expr>)> {
        let left = fixed_expr(left, fixed_names, scratch)?;
        let right = fixed_expr(right, fixed_names, scratch)?;
        Some((Box::new(left), Box::new(right)))
    }
    Some(match expr {
        ExprWire::Literal(StatValueWire::Fixed(value)) => Expr::Literal(StatValue::Fixed(*value)),
        ExprWire::Literal(_) => return None,
        ExprWire::Stat(name) => {
            if !fixed_names.contains(&name.as_str()) {
                return None;
            }
            Expr::Stat(scratch.stat(name)?)
        }
        ExprWire::Add { left, right } => {
            let (left, right) = pair(left, right, fixed_names, scratch)?;
            Expr::Add(left, right)
        }
        ExprWire::Subtract { left, right } => {
            let (left, right) = pair(left, right, fixed_names, scratch)?;
            Expr::Subtract(left, right)
        }
        ExprWire::Multiply { left, right } => {
            let (left, right) = pair(left, right, fixed_names, scratch)?;
            Expr::Multiply(left, right)
        }
        ExprWire::Divide { left, right } => {
            let (left, right) = pair(left, right, fixed_names, scratch)?;
            Expr::Divide(left, right)
        }
        ExprWire::Min { left, right } => {
            let (left, right) = pair(left, right, fixed_names, scratch)?;
            Expr::Min(left, right)
        }
        ExprWire::Max { left, right } => {
            let (left, right) = pair(left, right, fixed_names, scratch)?;
            Expr::Max(left, right)
        }
    })
}

/// B-L2: one authored length for one creature's base `Fixed` values.
fn resolve_length(
    length: &LengthWire,
    ruleset: &AuthoredRuleset,
    fixed: &[(String, Fx16_16)],
) -> Option<Fx16_16> {
    match length {
        LengthWire::Constant(value) => Some(*value),
        LengthWire::Stat(name) => {
            if let Some((_, value)) = fixed.iter().find(|(stat, _)| stat == name) {
                return Some(*value);
            }
            let derived_fixed = ruleset.stats.iter().any(|declaration| {
                declaration.name == *name
                    && matches!(declaration.kind, StatKindWire::Fixed)
                    && declaration.derived.is_some()
            });
            if derived_fixed {
                evaluate_derived_fixed(ruleset, fixed, name)
            } else {
                None
            }
        }
    }
}

/// B-L1: builds the mover for one placement and one creature's base `Fixed`
/// values, or `None` when the ruleset declares no `movement`.
///
/// Checks in order: position (`InvalidPosition`), speed (`InvalidSpeed`), step
/// (`InvalidStep`). Reads no RNG and interns nothing.
pub(crate) fn build_mover(
    ruleset: &AuthoredRuleset,
    placement: &AuthoredPlacement,
    fixed: &[(String, Fx16_16)],
) -> Result<Option<Mover>, CombatError> {
    let Some(movement) = ruleset.movement.as_deref() else {
        return Ok(None);
    };
    let position = placement.transform.position;
    if !point_inside(position) {
        return Err(CombatError::InvalidPosition {
            placement: placement.id,
        });
    }
    let speed = resolve_length(&movement.speed, ruleset, fixed)
        .filter(|value| (1..=MAX_SPEED_PER_TICK_RAW).contains(&value.to_raw()))
        .ok_or(CombatError::InvalidSpeed {
            placement: placement.id,
        })?;
    let step = resolve_length(&movement.turn.step, ruleset, fixed)
        .filter(|value| value.to_raw() >= 1)
        .ok_or(CombatError::InvalidStep {
            placement: placement.id,
        })?;
    Ok(Some(Mover {
        position,
        speed,
        step,
        route: None,
    }))
}

// ---------------------------------------------------------------------------
// Operations (M4, all-or-nothing)
// ---------------------------------------------------------------------------

/// Whether the entity's combatant or creature record is dead (Reading 6).
fn is_dead(world: &World, entity: EntityId) -> bool {
    world
        .combatants()
        .get(entity)
        .is_some_and(|combatant| combatant.dead())
        || world
            .creatures()
            .get(entity)
            .is_some_and(|record| record.dead())
}

impl World {
    /// M4 (B-O1): binds the navmesh revision routes are validated against.
    ///
    /// Binding the revision already bound changes nothing and returns `[]`.
    /// Otherwise every mover's route is cleared (position unchanged), each
    /// cleared mover's mirror is rewritten with zero velocity, and the cleared
    /// `(entity, tag)` pairs return in ascending [`EntityId`].
    pub fn bind_nav(&mut self, revision: NavRevision) -> Vec<(EntityId, RouteTag)> {
        if self.nav() == Some(revision) {
            return Vec::new();
        }
        self.set_nav(revision);
        let mut cleared = Vec::new();
        let routed: Vec<EntityId> = self
            .movers()
            .iter()
            .filter(|(_, mover)| mover.route.is_some())
            .map(|(entity, _)| entity)
            .collect();
        for entity in routed {
            let Some(mover) = self.movers_mut().get_mut(entity) else {
                continue;
            };
            let Some(route) = mover.route.take() else {
                continue;
            };
            cleared.push((entity, route.tag()));
            let mover = mover.clone();
            write_mirror(self, entity, &mover);
        }
        cleared.sort_by_key(|(entity, _)| *entity);
        cleared
    }

    /// M4 (B-O2): sets a mover's position and clears its route. A trusted host
    /// operation, never a client intent.
    ///
    /// Fails `AbsentEntity`, `NotMover`, then `OutOfBounds { index: 0 }`.
    pub fn place(&mut self, entity: EntityId, position: [Fx16_16; 3]) -> Result<(), MoveError> {
        if !self.contains(entity) {
            return Err(MoveError::AbsentEntity { entity });
        }
        if !self.movers().contains(entity) {
            return Err(MoveError::NotMover { entity });
        }
        if !point_inside(position) {
            return Err(MoveError::OutOfBounds { index: 0 });
        }
        let mover = self
            .movers_mut()
            .get_mut(entity)
            .expect("the mover presence is checked above");
        mover.position = position;
        mover.route = None;
        let mover = mover.clone();
        write_mirror(self, entity, &mover);
        Ok(())
    }

    /// M4 (B-O3): replaces a mover's route; returns the replaced route's tag.
    ///
    /// Fails `AbsentEntity`, `NotMover`, `DeadCombatant`, `TurnBased`, then the
    /// route checks of [`validate_route`] against the bound revision and the
    /// mover's position. A combatant of a real-time encounter and every
    /// non-participant may set a route.
    pub fn set_route(
        &mut self,
        entity: EntityId,
        route: MoveRoute,
    ) -> Result<Option<RouteTag>, MoveError> {
        if !self.contains(entity) {
            return Err(MoveError::AbsentEntity { entity });
        }
        let Some(mover) = self.movers().get(entity) else {
            return Err(MoveError::NotMover { entity });
        };
        if is_dead(self, entity) {
            return Err(MoveError::DeadCombatant { entity });
        }
        if self.combatants().contains(entity)
            && self
                .combat()
                .is_some_and(|state| state.mode == EncounterMode::TurnBased)
        {
            return Err(MoveError::TurnBased { entity });
        }
        validate_route(&route, self.nav(), mover.position)?;
        let mover = self
            .movers_mut()
            .get_mut(entity)
            .expect("the mover presence is checked above");
        let replaced = mover.route.replace(ActiveRoute {
            route,
            segment: 0,
            along: 0,
        });
        let mover = mover.clone();
        write_mirror(self, entity, &mover);
        Ok(replaced.map(|active| active.tag()))
    }

    /// M4 (B-O4): clears a mover's route; returns the cleared route's tag.
    ///
    /// Fails `AbsentEntity` then `NotMover`.
    pub fn clear_route(&mut self, entity: EntityId) -> Result<Option<RouteTag>, MoveError> {
        if !self.contains(entity) {
            return Err(MoveError::AbsentEntity { entity });
        }
        let Some(mover) = self.movers_mut().get_mut(entity) else {
            return Err(MoveError::NotMover { entity });
        };
        let cleared = mover.route.take();
        let mover = mover.clone();
        write_mirror(self, entity, &mover);
        Ok(cleared.map(|active| active.tag()))
    }
}

// ---------------------------------------------------------------------------
// The turn-based move (W4, C2-C4)
// ---------------------------------------------------------------------------

/// The observed result of one turn-based move (never persisted).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveOutcome {
    /// The creature that moved.
    pub actor: EntityId,
    /// The pool units spent.
    pub cost: u32,
    /// Where the creature stood before the move.
    pub from: [Fx16_16; 3],
    /// Where the creature stands after the move: the route's last point.
    pub to: [Fx16_16; 3],
}

/// C4: `ceil(measured / raw(step)) * step_cost` when it fits `u32` (B-C1).
///
/// `None` for a non-positive step or a cost above `u32::MAX`.
pub fn route_cost(measured: u64, step: Fx16_16, step_cost: u32) -> Option<u32> {
    let raw = u64::try_from(step.to_raw()).ok().filter(|raw| *raw > 0)?;
    let total = measured.div_ceil(raw).checked_mul(u64::from(step_cost))?;
    u32::try_from(total).ok()
}

/// W4's read-only twin of [`perform_move`]: the cost it would spend (B-C3).
///
/// Checks in order: `validate_action(EndTurn { actor })` wrapped as
/// `Combat(..)`; `NotMover` (no mover, or an encounter whose definition holds no
/// movement rules); [`validate_route`] against the bound revision and the
/// mover's position; then `CannotAfford` when the cost exceeds the actor's
/// balance of the movement pool. The cost is computed in `u64`, so a cost above
/// `u32::MAX` is reported as its true value.
pub fn validate_move(world: &World, actor: EntityId, route: &MoveRoute) -> Result<u32, MoveError> {
    validate_action(world, &CombatAction::EndTurn { actor }).map_err(MoveError::Combat)?;
    let movement = world
        .combat()
        .and_then(|state| state.definition.movement)
        .ok_or(MoveError::NotMover { entity: actor })?;
    let mover = world
        .movers()
        .get(actor)
        .ok_or(MoveError::NotMover { entity: actor })?;
    validate_route(route, world.nav(), mover.position)?;
    let measured = route_measure(movement.measure, &route.points)
        .expect("a validated route has a measured length");
    let step = u64::try_from(mover.step.to_raw()).map_or(1, |raw| raw.max(1));
    let cost = measured
        .div_ceil(step)
        .saturating_mul(u64::from(movement.step_cost));
    let current = world
        .combatants()
        .get(actor)
        .and_then(|combatant| pool_balance(combatant, movement.pool))
        .unwrap_or(0);
    if cost > u64::from(current) {
        return Err(MoveError::CannotAfford { cost, current });
    }
    Ok(u32::try_from(cost).expect("a cost within the balance fits u32"))
}

/// W4: one atomic turn-based move (B-C4).
///
/// Runs [`validate_move`], then spends exactly its cost from the movement pool,
/// sets the mover's position to the route's last point, clears its route and
/// writes the mirror with zero velocity. It keeps the turn, draws no RNG,
/// enqueues no event and never ends the turn. A failure changes nothing.
pub fn perform_move(
    world: &mut World,
    actor: EntityId,
    route: &MoveRoute,
) -> Result<MoveOutcome, MoveError> {
    let cost = validate_move(world, actor, route)?;
    let pool = world
        .combat()
        .and_then(|state| state.definition.movement)
        .ok_or(MoveError::NotMover { entity: actor })?
        .pool;
    let paid = world
        .combatants_mut()
        .get_mut(actor)
        .is_some_and(|combatant| spend_pool(combatant, pool, cost));
    if !paid {
        // Unreachable after `validate_move`; kept total instead of panicking.
        return Err(MoveError::NotMover { entity: actor });
    }
    let to = *route
        .points
        .last()
        .expect("a validated route has at least two points");
    let mover = world
        .movers_mut()
        .get_mut(actor)
        .ok_or(MoveError::NotMover { entity: actor })?;
    let from = mover.position;
    mover.position = to;
    mover.route = None;
    let mover = mover.clone();
    write_mirror(world, actor, &mover);
    Ok(MoveOutcome {
        actor,
        cost,
        from,
        to,
    })
}

// ---------------------------------------------------------------------------
// Stage 4 and the transitions that stop movers (W1-W3)
// ---------------------------------------------------------------------------

impl Mover {
    /// One stage-4 step (B-W2): walks the route by one tick of speed, or drops
    /// it when the mover is dead. Returns the fact it produced, if any.
    fn walk(&mut self, entity: EntityId, dead: bool) -> Option<MovementFact> {
        let active = self.route.as_mut()?;
        let tag = active.tag();
        if dead {
            self.route = None;
            return Some(MovementFact::Cancelled {
                entity,
                tag,
                reason: CancelReason::Dead,
            });
        }
        let mut budget = self.speed.to_raw() as u64;
        let points = &active.route.points;
        loop {
            let segment = usize::from(active.segment);
            let (from, to) = (points[segment], points[segment + 1]);
            let len = seg_len(from, to);
            let left = len - active.along;
            if budget < left {
                active.along += budget;
                self.position = point_at(from, to, len, active.along);
                return None;
            }
            budget -= left;
            active.segment += 1;
            active.along = 0;
            if usize::from(active.segment) == points.len() - 1 {
                self.position = to;
                self.route = None;
                return Some(MovementFact::Arrived { entity, tag });
            }
        }
    }
}

/// Tick stage 4 (B-W1, B-W2): every route advances by its mover's speed, in
/// ascending [`EntityId`] order, and every mover's mirror is rewritten.
///
/// Reads only fixed-point positions and writes the `f32` mirror; it never reads
/// a [`Transform`], touches a pool, an order, the timeline, the RNG or the
/// event queue (B-W3). Leftover budget at arrival is discarded.
pub(crate) fn movement_system(world: &mut World, facts: &mut crate::combat::TickFacts) {
    let mut entities: Vec<EntityId> = world.movers().iter().map(|(entity, _)| entity).collect();
    entities.sort();
    for entity in entities {
        let dead = is_dead(world, entity);
        let Some(mover) = world.movers_mut().get_mut(entity) else {
            continue;
        };
        let old = mover.position;
        if let Some(fact) = mover.walk(entity, dead) {
            facts.moves.push(fact);
        }
        let displacement = |axis: usize| {
            to_f32(Fx16_16::from_raw(
                mover.position[axis]
                    .to_raw()
                    .wrapping_sub(old[axis].to_raw()),
            ))
        };
        let velocity = [displacement(0), displacement(1), displacement(2)];
        let transform = mirror(mover, velocity);
        world.transforms_mut().insert(entity, transform);
    }
}

/// B-T1: clears the entity's route, rewrites its mirror with zero velocity and
/// returns the cleared tag. Does nothing, returning `None`, for a creature with
/// no mover or no route. The one way W3 clears routes.
pub(crate) fn stop(world: &mut World, entity: EntityId) -> Option<RouteTag> {
    let mover = world.movers_mut().get_mut(entity)?;
    let route = mover.route.take()?;
    let mover = mover.clone();
    write_mirror(world, entity, &mover);
    Some(route.tag())
}
