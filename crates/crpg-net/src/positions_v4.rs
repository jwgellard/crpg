//! Protocol v4 per-viewer position filter (T026p3, ADR-0036 L4/L5).
//!
//! [`PositionView`] is the companion of one
//! [`InterestView`](crate::interest::InterestView) and one
//! [`FactView`](crate::facts_v3::FactView): it remembers, per visible key, the
//! position the viewer was last told and the `NetId` it was told under. It mints
//! no id; every `NetId` is read from the one `InterestView` of the epoch.
//!
//! Disclosure is a reliable anchor, then unreliable samples. The lane-0 op
//! `Position { Some }` starts disclosure (and repeats it on a resend page);
//! `Position { None }` stops it while the entity stays visible. Values between
//! those two travel only in lane-1 samples ([`project_samples`]), which read
//! the current grant on every call, so a revoked position leaves the samples at
//! once and a position is sampled only after its anchor. A snapshot empties the
//! memory ([`project_snapshot_v4`]), so the next page re-anchors every granted
//! position. The module reads no `World`, RNG, clock or event state; the host
//! supplies every [`PositionRow`].

use std::collections::{BTreeMap, BTreeSet};

use crate::facts_v3::{FactContext, FactError, FactView, SnapshotProjectionV3};
use crate::interest::{EntityKey, InterestView, ViewCursor, ViewerContext};
use crate::movement_v4::SampleEntry;
use crate::protocol_v4::{DeltaOp, NetId, Point, PositionFact, SessionEpoch};

/// Most position rows one call reads: one per visible entity.
pub const MAX_POSITION_PAGE_OPS: usize = crate::protocol::MAX_VISIBLE_ENTITIES;

/// Most ops one [`project_page_v4`] result holds: the v3 page plus one
/// `Position` per visible entity. It is not a frame; split with
/// `codec_v4::frame_spans`.
pub const MAX_PAGE_OPS_V4: usize = crate::facts_v3::MAX_PAGE_OPS_V3 + MAX_POSITION_PAGE_OPS;

/// One mover's position for one viewer. Field order is contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PositionRow {
    /// Identity in authority terms.
    pub key: EntityKey,
    /// Whether this viewer may be told where the entity is. A `false` hides
    /// `point` and `moving` completely.
    pub granted: bool,
    /// Where the entity is, within U3 by its type.
    pub point: Point,
    /// Whether the entity currently has a route.
    pub moving: bool,
}

/// What one viewer was last told of each position, with the `NetId` it was
/// told under.
///
/// Not `Serialize`: a saved view would restore stale authorization. There is
/// deliberately no restore constructor, and it mints nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositionView {
    epoch: SessionEpoch,
    told: BTreeMap<EntityKey, (NetId, PositionFact)>,
}

impl PositionView {
    /// Opens an empty position view for one session epoch.
    pub fn new(epoch: SessionEpoch) -> Self {
        Self {
            epoch,
            told: BTreeMap::new(),
        }
    }

    /// Returns the epoch this view was opened for.
    pub fn epoch(&self) -> SessionEpoch {
        self.epoch
    }
}

/// One filtered v4 page plus the view position after the call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageV4 {
    /// The v3 page's ops wrapped in `V3`, then the `Position` ops; at most
    /// [`MAX_PAGE_OPS_V4`]. Not a frame.
    pub ops: Vec<DeltaOp>,
    /// `ops[..enters]` are the `EntityEnter`s; history goes at this index.
    pub enters: usize,
    /// The interest view's position after the call.
    pub cursor: ViewCursor,
}

/// Checks the row bounds shared by [`project_page_v4`] and
/// [`project_samples`]: at most [`MAX_POSITION_PAGE_OPS`] rows, one per key.
fn check_rows(rows: &[PositionRow]) -> Result<(), FactError> {
    if rows.len() > MAX_POSITION_PAGE_OPS {
        return Err(FactError::TooManyFacts);
    }
    let mut seen = BTreeSet::new();
    for row in rows {
        if !seen.insert(row.key) {
            return Err(FactError::DuplicateFact);
        }
    }
    Ok(())
}

/// Projects one v4 page: the v3 page, then the position ops.
///
/// Checks, in order, before anything changes: `positions`' epoch against
/// `facts`' ([`FactError::WrongEpoch`]), more than
/// [`MAX_POSITION_PAGE_OPS`] rows ([`FactError::TooManyFacts`]), two rows with
/// one key ([`FactError::DuplicateFact`]), then
/// [`project_page`](crate::facts_v3::project_page) with its error unchanged.
/// Every `Err` leaves `view`, `facts` and `positions` equal to their pre-call
/// values; after the last check nothing can fail.
///
/// Then `positions` forgets every key whose id is no longer the one it was
/// told under (no op), and, for each visible key in ascending `NetId`, one op
/// at most: a granted row not yet told is anchored, a told key whose row is no
/// longer granted (or is gone) is withdrawn with `None`, and a resend page
/// re-anchors every granted position. A changed point is the samples' job.
pub fn project_page_v4(
    view: &mut InterestView,
    facts: &mut FactView,
    positions: &mut PositionView,
    state: &ViewerContext<'_>,
    context: &FactContext<'_>,
    rows: &[PositionRow],
) -> Result<PageV4, FactError> {
    if positions.epoch != facts.epoch() {
        return Err(FactError::WrongEpoch);
    }
    check_rows(rows)?;
    let page = crate::facts_v3::project_page(view, facts, state, context)?;

    // From here nothing can fail.
    positions
        .told
        .retain(|key, (net, _)| view.net_id(*key) == Some(*net));

    let by_key: BTreeMap<EntityKey, &PositionRow> = rows.iter().map(|r| (r.key, r)).collect();
    let mut visible: Vec<(NetId, EntityKey)> = state
        .facts
        .iter()
        .filter_map(|fact| view.net_id(fact.key).map(|net| (net, fact.key)))
        .collect();
    visible.sort();

    let mut ops: Vec<DeltaOp> = page.ops.into_iter().map(DeltaOp::from).collect();
    for (net, key) in visible {
        let image = by_key
            .get(&key)
            .filter(|row| row.granted)
            .map(|row| PositionFact {
                point: row.point,
                moving: row.moving,
            });
        let remembered = positions.told.contains_key(&key);
        match (image, remembered) {
            (Some(image), false) => {
                ops.push(DeltaOp::Position {
                    entity: net,
                    position: Some(image),
                });
                positions.told.insert(key, (net, image));
            }
            (None, true) => {
                ops.push(DeltaOp::Position {
                    entity: net,
                    position: None,
                });
                positions.told.remove(&key);
            }
            (Some(image), true) if context.resend => {
                ops.push(DeltaOp::Position {
                    entity: net,
                    position: Some(image),
                });
                positions.told.insert(key, (net, image));
            }
            _ => {}
        }
    }

    Ok(PageV4 {
        ops,
        enters: page.enters,
        cursor: page.cursor,
    })
}

/// Projects one tick's position samples for one viewer.
///
/// Checks, in order: `positions`' epoch against `view`'s
/// ([`FactError::WrongEpoch`]), the row bounds ([`FactError::TooManyFacts`],
/// [`FactError::DuplicateFact`]); an `Err` leaves `positions` unchanged
/// (`view` is read only). Then, for each remembered key in ascending `NetId`
/// whose id is still current and whose row is granted now, one entry when the
/// value differs from what the viewer was told (or on `refresh`); the value
/// is then remembered. Every other remembered key gives no entry and keeps its
/// memory. No key is ever added: a position is sampled only after its anchor,
/// and the current grant is read on every call, so revocation is immediate.
/// The result may exceed one datagram; split it with
/// `movement_v4::sample_datagrams`.
pub fn project_samples(
    view: &InterestView,
    positions: &mut PositionView,
    rows: &[PositionRow],
    refresh: bool,
) -> Result<Vec<SampleEntry>, FactError> {
    if positions.epoch != view.epoch() {
        return Err(FactError::WrongEpoch);
    }
    check_rows(rows)?;

    // From here nothing can fail.
    let by_key: BTreeMap<EntityKey, &PositionRow> = rows.iter().map(|r| (r.key, r)).collect();
    let mut remembered: Vec<(NetId, EntityKey, PositionFact)> = positions
        .told
        .iter()
        .map(|(key, (net, told))| (*net, *key, *told))
        .collect();
    remembered.sort_by_key(|(net, _, _)| *net);

    let mut entries = Vec::new();
    for (net, key, told) in remembered {
        if view.net_id(key) != Some(net) {
            continue;
        }
        let Some(row) = by_key.get(&key).filter(|row| row.granted) else {
            continue;
        };
        let fact = PositionFact {
            point: row.point,
            moving: row.moving,
        };
        if fact != told || refresh {
            entries.push(SampleEntry {
                entity: net,
                point: fact.point,
                moving: fact.moving,
            });
            positions.told.insert(key, (net, fact));
        }
    }
    Ok(entries)
}

/// Projects one v4 replacement snapshot: the v3 snapshot, then an emptied
/// position memory.
///
/// Checks `positions`' epoch against `facts`' ([`FactError::WrongEpoch`]),
/// then runs [`project_snapshot_v3`](crate::facts_v3::project_snapshot_v3)
/// with its error unchanged; an `Err` changes nothing. On `Ok` it returns the
/// v3 result unchanged and forgets every position, so the next
/// [`project_page_v4`] anchors every granted visible position and
/// [`project_samples`] returns nothing until then.
pub fn project_snapshot_v4(
    view: &mut InterestView,
    facts: &mut FactView,
    positions: &mut PositionView,
    state: &ViewerContext<'_>,
    context: &FactContext<'_>,
) -> Result<SnapshotProjectionV3, FactError> {
    if positions.epoch != facts.epoch() {
        return Err(FactError::WrongEpoch);
    }
    let snapshot = crate::facts_v3::project_snapshot_v3(view, facts, state, context)?;
    positions.told.clear();
    Ok(snapshot)
}
