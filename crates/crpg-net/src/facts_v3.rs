//! Protocol v3 per-viewer fact filter (T140c): grants, memory, change detection.
//!
//! [`FactView`] is the companion of one [`InterestView`]: it decides which of
//! the T140b fact ops (`Encounter`, `Timeline`, `EntityFacts`) a viewer
//! receives, filters every host-supplied value by grant, and remembers what
//! the viewer was last told so an unchanged image is not re-sent. It mints no
//! id: every `NetId` is resolved through the one `InterestView` of the epoch,
//! and it is driven only through [`project_page`], which validates before
//! either view mutates, so a call is all-or-nothing. Absent means withheld: a
//! `None` is "not disclosed", never zero.
//!
//! The host-fact helpers [`clock_op`] and [`host_facts_op`] are stateless and
//! build the two per-viewer host ops from the host's values and that viewer's
//! [`HostGrants`]. The module reads no `World`, RNG, clock or event state and
//! adds no wire form.

use core::fmt;
use std::collections::{BTreeMap, BTreeSet};

use crpg_core::Ulid;

use crate::interest::{EntityKey, InterestError, InterestView, ViewCursor, ViewerContext};
use crate::protocol_v3::{
    check_effects, check_pools, check_stats, check_tags, AutoPauseToggles, ClockFact, DeltaOp,
    EffectFact, EncounterFacts, EncounterMode, FactSet, HostFacts, NetId, OrderFact, PauseReason,
    PauseRights, PoolFact, SessionEpoch, StatFact, TimelineEntry, MAX_TIMELINE_ENTRIES,
};
use crate::transfer_v3::{SnapshotBodyV3, SnapshotEntityV3};

/// Most fact ops one page holds: one `Encounter`, one `Timeline` and one
/// `EntityFacts` per visible row.
pub const MAX_FACT_PAGE_OPS: usize = crate::protocol::MAX_VISIBLE_ENTITIES + 2;

/// Most ops one [`project_page`] result holds: the view's page plus the fact
/// ops. It is not a frame; split with `codec_v3::frame_spans`.
pub const MAX_PAGE_OPS_V3: usize = crate::interest::MAX_PAGE_OPS + MAX_FACT_PAGE_OPS;

/// Per-row fact grants. `Default` denies everything, and `order` stays off
/// until the host grants it (to the controlling peer only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FactMask {
    /// The row's pools may be disclosed.
    pub pools: bool,
    /// The row's attached effects may be disclosed.
    pub effects: bool,
    /// The row's standing order may be disclosed.
    pub order: bool,
    /// The row's stat values may be disclosed.
    pub stats: bool,
    /// The row's record tags may be disclosed.
    pub tags: bool,
}

/// A standing order in authority terms: the target is a runtime key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrderInput {
    /// Authored ability identity.
    pub ability: Ulid,
    /// The order's target; it must be visible to the viewer to be named.
    pub target: EntityKey,
    /// Whether the order repeats.
    pub repeat: bool,
}

/// One entity's facts for one viewer. Field order is contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactRow {
    /// Identity in authority terms.
    pub key: EntityKey,
    /// What this viewer may see of the row.
    pub fields: FactMask,
    /// The pools; disclosure still needs `fields.pools`.
    pub pools: Vec<PoolFact>,
    /// The attached effects; disclosure still needs `fields.effects`.
    pub effects: Vec<EffectFact>,
    /// The standing order; disclosure still needs `fields.order`.
    pub order: Option<OrderInput>,
    /// The stat values; disclosure still needs `fields.stats`.
    pub stats: Vec<StatFact>,
    /// The record tags; disclosure still needs `fields.tags`.
    pub tags: Vec<String>,
}

/// Per-viewer encounter grants. `Default` denies everything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EncounterGrants {
    /// The time mode may be disclosed.
    pub mode: bool,
    /// The pending mode may be disclosed.
    pub pending_mode: bool,
    /// The round clock may be disclosed.
    pub clock: bool,
    /// The timeline may be disclosed.
    pub timeline: bool,
}

/// The encounter in authority terms.
#[derive(Debug, Clone, Copy)]
pub struct EncounterInput<'a> {
    /// The area whose encounter this is.
    pub area: Ulid,
    /// The time mode.
    pub mode: EncounterMode,
    /// The pending mode, if one is queued.
    pub pending_mode: Option<EncounterMode>,
    /// The round clock, if the mode has one.
    pub clock: Option<ClockFact>,
    /// The timeline as `(key, entity)` in any order; at most
    /// [`MAX_TIMELINE_ENTRIES`], one entry per entity key.
    pub timeline: &'a [(i32, EntityKey)],
}

/// The fact-side context of one call. Field order is contract.
#[derive(Debug, Clone, Copy)]
pub struct FactContext<'a> {
    /// At most `MAX_VISIBLE_ENTITIES`, one row per key, in any order.
    pub rows: &'a [FactRow],
    /// The encounter, if the authority has one.
    pub encounter: Option<EncounterInput<'a>>,
    /// What this viewer may see of the encounter.
    pub grants: EncounterGrants,
    /// This page only: re-emit every non-withheld image.
    pub resend: bool,
}

/// A rejected [`project_page`] call. Every variant leaves both views intact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactError {
    /// The state's epoch or the interest view's epoch is not the fact view's.
    WrongEpoch,
    /// More than `MAX_VISIBLE_ENTITIES` rows, or more than
    /// [`MAX_TIMELINE_ENTRIES`] timeline entries.
    TooManyFacts,
    /// Two rows, or two timeline entries, share one [`EntityKey`].
    DuplicateFact,
    /// `InterestView::project_delta` refused the state.
    Interest(InterestError),
}

impl fmt::Display for FactError {
    /// Renders `<VariantName> at projection/facts`, without payloads.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongEpoch => write!(f, "WrongEpoch at projection/facts"),
            Self::TooManyFacts => write!(f, "TooManyFacts at projection/facts"),
            Self::DuplicateFact => write!(f, "DuplicateFact at projection/facts"),
            Self::Interest(_) => write!(f, "Interest at projection/facts"),
        }
    }
}

impl std::error::Error for FactError {}

/// What one viewer was last told: the encounter image, the timeline image and
/// each visible key's fact set with the `NetId` it was told under.
///
/// Not `Serialize`: a saved view would restore stale authorization. There is
/// deliberately no restore constructor, and it mints nothing; it resolves
/// every key through the one `InterestView` of the epoch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactView {
    epoch: SessionEpoch,
    encounter: EncounterFacts,
    timeline: Option<Vec<TimelineEntry>>,
    rows: BTreeMap<EntityKey, (NetId, FactSet)>,
}

impl FactView {
    /// Opens an empty fact view for one session epoch.
    pub fn new(epoch: SessionEpoch) -> Self {
        Self {
            epoch,
            encounter: EncounterFacts::default(),
            timeline: None,
            rows: BTreeMap::new(),
        }
    }

    /// Returns the epoch this view was opened for.
    pub fn epoch(&self) -> SessionEpoch {
        self.epoch
    }
}

/// One filtered v3 page plus the view position after the call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageV3 {
    /// The view's ops converted, then the fact ops; at most
    /// [`MAX_PAGE_OPS_V3`]. Not a frame.
    pub ops: Vec<DeltaOp>,
    /// `ops[..enters]` are the `EntityEnter`s; history goes at this index.
    pub enters: usize,
    /// The interest view's position after the call.
    pub cursor: ViewCursor,
}

/// Projects one v3 page: the interest view's delta, then the fact ops.
///
/// Checks, in order, before anything changes: epochs ([`FactError::WrongEpoch`]),
/// bounds ([`FactError::TooManyFacts`]), duplicate keys
/// ([`FactError::DuplicateFact`]), then the interest projection
/// ([`FactError::Interest`]). Every `Err` leaves `view` and `facts` equal to
/// their pre-call values. After the interest projection succeeds nothing can
/// fail. Emission order after the view's ops: `Encounter`, `Timeline`, then
/// `EntityFacts` in ascending `NetId`.
pub fn project_page(
    view: &mut InterestView,
    facts: &mut FactView,
    state: &ViewerContext<'_>,
    context: &FactContext<'_>,
) -> Result<PageV3, FactError> {
    if state.epoch != facts.epoch || view.epoch() != facts.epoch {
        return Err(FactError::WrongEpoch);
    }
    let timeline_len = context.encounter.map_or(0, |e| e.timeline.len());
    if context.rows.len() > crate::protocol::MAX_VISIBLE_ENTITIES
        || timeline_len > MAX_TIMELINE_ENTRIES
    {
        return Err(FactError::TooManyFacts);
    }
    let mut seen = BTreeSet::new();
    for row in context.rows {
        if !seen.insert(row.key) {
            return Err(FactError::DuplicateFact);
        }
    }
    if let Some(encounter) = &context.encounter {
        let mut seen = BTreeSet::new();
        for (_, key) in encounter.timeline {
            if !seen.insert(*key) {
                return Err(FactError::DuplicateFact);
            }
        }
    }
    let page = view.project_delta(state).map_err(FactError::Interest)?;

    // From here nothing can fail.
    let encounter_image = encounter_image(state, context);
    let timeline_image = timeline_image(view, state, context);

    // Visible keys: a state row's key resolves only if it is live after the
    // projection, and live is exactly visible.
    let mut visible: BTreeMap<EntityKey, NetId> = BTreeMap::new();
    for fact in state.facts {
        if let Some(net) = view.net_id(fact.key) {
            visible.insert(fact.key, net);
        }
    }
    let rows: BTreeMap<EntityKey, &FactRow> = context.rows.iter().map(|r| (r.key, r)).collect();

    // C-5: forget every remembered key that left, retired or re-entered.
    facts
        .rows
        .retain(|key, (net, _)| view.net_id(*key) == Some(*net));

    let mut ops: Vec<DeltaOp> = page.ops.into_iter().map(DeltaOp::from).collect();

    if encounter_image != facts.encounter || (context.resend && !encounter_image.is_withheld()) {
        ops.push(DeltaOp::Encounter(encounter_image));
    }
    if timeline_image != facts.timeline || (context.resend && timeline_image.is_some()) {
        ops.push(DeltaOp::Timeline {
            entries: timeline_image.clone(),
        });
    }

    let mut renders: Vec<(NetId, EntityKey, FactSet)> = visible
        .iter()
        .map(|(key, net)| {
            let image = rows
                .get(key)
                .map_or_else(FactSet::default, |row| row_image(view, row));
            (*net, *key, image)
        })
        .collect();
    renders.sort_by_key(|(net, _, _)| *net);
    let default_image = FactSet::default();
    for (net, key, image) in &renders {
        let remembered = facts
            .rows
            .get(key)
            .map_or(&default_image, |(_, remembered)| remembered);
        if image != remembered || (context.resend && !image.is_withheld()) {
            ops.push(DeltaOp::EntityFacts {
                entity: *net,
                facts: image.clone(),
            });
        }
    }

    facts.encounter = encounter_image;
    facts.timeline = timeline_image;
    for (net, key, image) in renders {
        facts.rows.insert(key, (net, image));
    }

    Ok(PageV3 {
        ops,
        enters: page.enters,
        cursor: page.cursor,
    })
}

/// The encounter as this viewer may see it (C-2).
fn encounter_image(state: &ViewerContext<'_>, context: &FactContext<'_>) -> EncounterFacts {
    let Some(encounter) = context.encounter else {
        return EncounterFacts::default();
    };
    if encounter.area != state.interest.area {
        return EncounterFacts::default();
    }
    let grants = context.grants;
    EncounterFacts {
        mode: grants.mode.then_some(encounter.mode),
        pending_mode: grants.pending_mode.then_some(encounter.pending_mode),
        clock: (grants.clock && encounter.clock.is_none_or(|c| c.check().is_ok()))
            .then_some(encounter.clock),
    }
}

/// The timeline as this viewer may see it (C-3): ascending `(key, EntityKey)`,
/// hidden entities dropped.
fn timeline_image(
    view: &InterestView,
    state: &ViewerContext<'_>,
    context: &FactContext<'_>,
) -> Option<Vec<TimelineEntry>> {
    let encounter = context.encounter?;
    if encounter.area != state.interest.area || !context.grants.timeline {
        return None;
    }
    let mut sorted: Vec<(i32, EntityKey)> = encounter.timeline.to_vec();
    sorted.sort();
    Some(
        sorted
            .into_iter()
            .filter_map(|(key, entity)| {
                view.net_id(entity)
                    .map(|entity| TimelineEntry { key, entity })
            })
            .collect(),
    )
}

/// One visible row's fact set (C-4). A family is withheld when its grant is
/// off or its list breaks a wire bound; the order is withheld when its target
/// has no `NetId` for this viewer.
fn row_image(view: &InterestView, row: &FactRow) -> FactSet {
    let fields = row.fields;
    FactSet {
        pools: (fields.pools && check_pools(&row.pools).is_ok()).then(|| row.pools.clone()),
        effects: (fields.effects && check_effects(&row.effects).is_ok())
            .then(|| row.effects.clone()),
        order: if fields.order {
            match &row.order {
                None => Some(None),
                Some(order) => view.net_id(order.target).map(|target| {
                    Some(OrderFact {
                        ability: order.ability,
                        target,
                        repeat: order.repeat,
                    })
                }),
            }
        } else {
            None
        },
        stats: (fields.stats && check_stats(&row.stats).is_ok()).then(|| row.stats.clone()),
        tags: (fields.tags && check_tags(&row.tags).is_ok()).then(|| row.tags.clone()),
    }
}

/// Per-viewer host-fact grants. `Default` denies everything.
///
/// Whether the session is paused and the viewer's own pause rights are never
/// granted: every client already observes a pause, and the rights describe
/// only that viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HostGrants {
    /// The pause reason may be disclosed.
    pub pause_reason: bool,
    /// The auto-pause toggles may be disclosed.
    pub auto_pause: bool,
    /// The turn timer may be disclosed.
    pub turn_timer: bool,
}

/// The host's values for one viewer. Field order is contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostInput {
    /// Why the host is paused; [`None`] is running.
    pub paused: Option<PauseReason>,
    /// This viewer's own pause rights.
    pub rights: PauseRights,
    /// The auto-pause toggles.
    pub auto_pause: AutoPauseToggles,
    /// Remaining turn-timer milliseconds; [`None`] is no timer running.
    pub turn_timer_ms: Option<u32>,
}

/// Builds the viewer's `Clock` op. The paused state is always disclosed; the
/// reason only under [`HostGrants::pause_reason`].
pub fn clock_op(input: &HostInput, grants: HostGrants) -> DeltaOp {
    DeltaOp::Clock {
        paused: input
            .paused
            .map(|reason| grants.pause_reason.then_some(reason)),
    }
}

/// Builds the viewer's `HostFacts` op. The rights are always disclosed; the
/// toggles and the timer only under their grants.
pub fn host_facts_op(input: &HostInput, grants: HostGrants) -> DeltaOp {
    DeltaOp::HostFacts(HostFacts {
        rights: input.rights,
        auto_pause: grants.auto_pause.then_some(input.auto_pause),
        turn_timer_ms: grants.turn_timer.then_some(input.turn_timer_ms),
    })
}

/// One filtered v3 snapshot body plus the view position after the call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotProjectionV3 {
    /// The `0x82` body: the view's entities and turn, each entity's row
    /// image, and the encounter and timeline images. Host facts are never in
    /// the body.
    pub body: SnapshotBodyV3,
    /// The interest view's position after the call.
    pub cursor: ViewCursor,
}

/// Projects one v3 replacement snapshot: the interest view's snapshot, then
/// the fact images.
///
/// Runs the [`project_page`] checks (epochs, bounds, duplicate keys), then
/// `InterestView::project_snapshot` ([`FactError::Interest`]). Every `Err`
/// leaves `view` and `facts` equal to their pre-call values. The `FactView`
/// then remembers exactly the images in the body, so the next
/// [`project_page`] emits only differences. The body always passes
/// `transfer_v3::encode_body`.
pub fn project_snapshot_v3(
    view: &mut InterestView,
    facts: &mut FactView,
    state: &ViewerContext<'_>,
    context: &FactContext<'_>,
) -> Result<SnapshotProjectionV3, FactError> {
    if state.epoch != facts.epoch || view.epoch() != facts.epoch {
        return Err(FactError::WrongEpoch);
    }
    let timeline_len = context.encounter.map_or(0, |e| e.timeline.len());
    if context.rows.len() > crate::protocol::MAX_VISIBLE_ENTITIES
        || timeline_len > MAX_TIMELINE_ENTRIES
    {
        return Err(FactError::TooManyFacts);
    }
    let mut seen = BTreeSet::new();
    for row in context.rows {
        if !seen.insert(row.key) {
            return Err(FactError::DuplicateFact);
        }
    }
    if let Some(encounter) = &context.encounter {
        let mut seen = BTreeSet::new();
        for (_, key) in encounter.timeline {
            if !seen.insert(*key) {
                return Err(FactError::DuplicateFact);
            }
        }
    }
    let snapshot = view.project_snapshot(state).map_err(FactError::Interest)?;

    // From here nothing can fail.
    let encounter_image = encounter_image(state, context);
    let timeline_image = timeline_image(view, state, context);
    let rows: BTreeMap<EntityKey, &FactRow> = context.rows.iter().map(|r| (r.key, r)).collect();

    let mut entities: Vec<SnapshotEntityV3> = Vec::with_capacity(snapshot.body.entities.len());
    let mut remembered: BTreeMap<EntityKey, (NetId, FactSet)> = BTreeMap::new();
    for entity in snapshot.body.entities {
        let key = view.entity(entity.entity);
        let image = key
            .and_then(|key| rows.get(&key))
            .map_or_else(FactSet::default, |row| row_image(view, row));
        if let Some(key) = key {
            remembered.insert(key, (entity.entity, image.clone()));
        }
        entities.push(SnapshotEntityV3 {
            entity: entity.entity,
            health: entity.health,
            facts: image,
        });
    }

    facts.encounter = encounter_image;
    facts.timeline = timeline_image.clone();
    facts.rows = remembered;

    Ok(SnapshotProjectionV3 {
        body: SnapshotBodyV3 {
            entities,
            turn: snapshot.body.turn,
            encounter: encounter_image,
            timeline: timeline_image,
        },
        cursor: snapshot.cursor,
    })
}
