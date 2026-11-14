//! Protocol v3 session state and `World` reading (T141c, ADR-0029 N1-N3).
//!
//! A session bound with `Host::bind_v3_peer` carries a [`V3Session`]: the
//! per-epoch `InterestView` and `FactView` (T140c), the operator's fact grants
//! and the viewer's pause rights, and the paced outbox of staged `0x82` frames
//! (H-7). Everything here is pure data and pure functions: the host reads
//! `World` into [`WorldRead`] on every call (Reading 8), so no view of the
//! world is ever stored, and the ledgers that pacing moves frames into stay in
//! `host.rs`.

use std::collections::VecDeque;

use crpg_core::{EntityId, Ulid};
use crpg_net::facts_v3::{
    EncounterGrants, EncounterInput, FactContext, FactMask, FactRow, FactView, HostGrants,
    OrderInput,
};
use crpg_net::interest::{
    EntityFact, EntityKey, FieldMask, InterestSet, InterestView, TurnState, ViewerContext,
};
use crpg_net::protocol::SessionEpoch;
use crpg_net::protocol_transfer::HealthFact;
use crpg_net::protocol_v3::{
    AutoPauseToggles, ClockFact, DeltaOp, EffectFact, EncounterMode, PauseRights, PoolFact,
    StatFact, StatValue,
};
use crpg_sim::World;

use crate::capture::CapturedView;
use crate::host::{
    DisclosureGrants, EntityFactGrant, FactGrants, HOST_UNBOUND_AREA, POLICY_V3_IN_FLIGHT_BYTES,
    POLICY_V3_IN_FLIGHT_FRAMES, POLICY_V3_THROTTLE_TICKS,
};

/// Most entries one staged plan holds in the outbox (H-7): a `Cancel`, the D5
/// plan's 88 frames and the host-facts delta.
pub(crate) const OUTBOX_MAX_ENTRIES: usize = 90;
/// Most encoded bytes one staged plan holds in the outbox: the D5 plan's
/// 2,851,124 bytes plus a 32-byte `Cancel` and a 992-byte delta allowance.
pub(crate) const OUTBOX_MAX_BYTES: usize = 2_852_148;

/// Whether a plan of `entries` frames and `bytes` encoded bytes fits the
/// outbox bound (C-11).
pub(crate) fn outbox_fits(entries: usize, bytes: usize) -> bool {
    entries <= OUTBOX_MAX_ENTRIES && bytes <= OUTBOX_MAX_BYTES
}

/// One staged frame waiting for egress room. `event_seq` is `Some` for a
/// lane-0 delta and `None` for a `0x82` transfer frame; a delta took its
/// sequence when it was built, not when pacing logs it (C-6).
#[derive(Debug)]
pub(crate) struct OutboxEntry {
    pub(crate) event_seq: Option<u64>,
    pub(crate) bytes: Vec<u8>,
}

/// What the host's facts image says; the countdown alone never changes it
/// (D-12). Compared against the session's last sent image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HostFactsImage {
    pub(crate) rights: PauseRights,
    pub(crate) auto_pause: Option<AutoPauseToggles>,
    pub(crate) timer_epoch: Option<u64>,
}

/// What the in-flight throttle says of one publication (E-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Gate {
    /// The frames may be logged.
    Open,
    /// Too soon since the last page, or the deltas already ended.
    Held,
    /// The frames would pass the held-delta budget.
    Over,
}

/// A v3 session's delta allowance from a staging until its transfer ends
/// (H-8, D54, E-2): one page per [`POLICY_V3_THROTTLE_TICKS`] ticks, within
/// the receiver's held-delta frame and byte budget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Throttle {
    /// The server tick of the staging, then of the last logged frames.
    last_page: u64,
    /// Delta frames logged since the staging, C-6's frame included.
    frames: usize,
    /// Encoded bytes of those frames.
    bytes: usize,
    /// A page the budget refused ended the deltas.
    closed: bool,
}

impl Throttle {
    /// A throttle restarted by a staging at `tick` that logged `frames`
    /// frames of `bytes` bytes (C-6's host-facts delta).
    pub(crate) fn new(tick: u64, frames: usize, bytes: usize) -> Self {
        Self {
            last_page: tick,
            frames,
            bytes,
            closed: false,
        }
    }

    /// Whether `frames` frames of `bytes` bytes may be logged at `tick`: not
    /// before `last_page + POLICY_V3_THROTTLE_TICKS`, never once closed, and
    /// within both in-flight budgets (an exact fit is allowed).
    pub(crate) fn gate(&self, tick: u64, frames: usize, bytes: usize) -> Gate {
        if self.closed || tick < self.last_page.saturating_add(POLICY_V3_THROTTLE_TICKS) {
            return Gate::Held;
        }
        if self.frames.saturating_add(frames) > POLICY_V3_IN_FLIGHT_FRAMES
            || self.bytes.saturating_add(bytes) > POLICY_V3_IN_FLIGHT_BYTES
        {
            return Gate::Over;
        }
        Gate::Open
    }

    /// Counts logged frames; a page with none changes nothing.
    pub(crate) fn record(&mut self, tick: u64, frames: usize, bytes: usize) {
        if frames > 0 {
            self.last_page = tick;
            self.frames += frames;
            self.bytes += bytes;
        }
    }

    /// Ends the session's deltas until its transfer ends.
    pub(crate) fn close(&mut self) {
        self.closed = true;
    }
}

/// One v3 session's state beside the shared [`crate::host`] session ledgers.
#[derive(Debug)]
pub(crate) struct V3Session {
    /// The epoch's one id-minting view (Reading 8).
    pub(crate) view: InterestView,
    /// What the viewer was last told of facts and the encounter.
    pub(crate) facts: FactView,
    /// The operator's fact grants, sorted by entity.
    pub(crate) fact_grants: FactGrants,
    /// The viewer's own pause rights.
    pub(crate) rights: PauseRights,
    /// The staged plan not yet logged: at most [`OUTBOX_MAX_ENTRIES`].
    pub(crate) outbox: VecDeque<OutboxEntry>,
    /// The last `Clock` op staged for the viewer (D-12).
    pub(crate) sent_clock: Option<DeltaOp>,
    /// The last host-facts image staged for the viewer (D-12).
    pub(crate) sent_host_facts: Option<HostFactsImage>,
    /// The server tick of the last logged `Clock` (the heartbeat's clock,
    /// E-1).
    pub(crate) last_clock_tick: u64,
    /// The in-flight throttle, set by a staging and cleared when the
    /// transfer is `Published` (E-2).
    pub(crate) throttle: Option<Throttle>,
}

impl V3Session {
    /// A fresh session for one epoch: empty views, nothing sent.
    pub(crate) fn new(epoch: [u8; 16], fact_grants: FactGrants, rights: PauseRights) -> Self {
        Self {
            view: InterestView::new(SessionEpoch(epoch)),
            facts: FactView::new(SessionEpoch(epoch)),
            fact_grants,
            rights,
            outbox: VecDeque::new(),
            sent_clock: None,
            sent_host_facts: None,
            last_clock_tick: 0,
            throttle: None,
        }
    }
}

/// One planned staging (C-5 steps 1 to 4): everything the commit installs.
/// Built by reading only, so a failed plan changes nothing.
#[derive(Debug)]
pub(crate) struct V3Stage {
    /// The projected clone of the interest view.
    pub(crate) view: InterestView,
    /// The projected clone of the fact view.
    pub(crate) facts: FactView,
    /// The `Cancel` of the in-flight transfer, if any.
    pub(crate) cancel: Option<Vec<u8>>,
    /// The `Begin` and `Chunk` frames of the new transfer.
    pub(crate) frames: Vec<Vec<u8>>,
    /// The host-facts delta (C-6) and the `event_seq` it took.
    pub(crate) delta: Vec<u8>,
    pub(crate) delta_seq: u64,
    /// The id the new transfer takes.
    pub(crate) transfer: u64,
    /// The server tick the plan was cut at.
    pub(crate) tick: u64,
    /// The `Clock` op and facts image the delta carries.
    pub(crate) clock: DeltaOp,
    pub(crate) image: HostFactsImage,
}

/// One planned publication page (D-7) for one v3 session: the projected
/// clones, the encoded frames (each with the `event_seq` it took) and what the
/// page told the viewer of the host. Built by reading only; committing it
/// installs the clones, logs the frames and remembers the sent host facts.
#[derive(Debug)]
pub(crate) struct PagePlan {
    /// The projected clone of the interest view.
    pub(crate) view: InterestView,
    /// The projected clone of the fact view.
    pub(crate) facts: FactView,
    /// One encoded frame per `frame_spans` span, ascending `event_seq`.
    pub(crate) frames: Vec<(u64, Vec<u8>)>,
    /// The `Clock` op the page carries, if it carries one.
    pub(crate) clock: Option<DeltaOp>,
    /// The host-facts image the page carries, if it carries `HostFacts`.
    pub(crate) image: Option<HostFactsImage>,
    /// The server tick the page was cut at.
    pub(crate) tick: u64,
    /// The session's capture view of an accepted command (D-10).
    pub(crate) capture: Option<CapturedView>,
}

/// The sim mode of a wire mode.
pub(crate) fn sim_mode(mode: EncounterMode) -> crpg_sim::EncounterMode {
    match mode {
        EncounterMode::TurnBased => crpg_sim::EncounterMode::TurnBased,
        EncounterMode::RealTime => crpg_sim::EncounterMode::RealTime,
    }
}

/// The host-fact image of one viewer's grants (D-12).
pub(crate) fn host_facts_image(
    rights: PauseRights,
    auto_pause: AutoPauseToggles,
    grants: HostGrants,
    timer_epoch: u64,
) -> HostFactsImage {
    HostFactsImage {
        rights,
        auto_pause: grants.auto_pause.then_some(auto_pause),
        timer_epoch: grants.turn_timer.then_some(timer_epoch),
    }
}

/// The wire mode of a sim mode.
pub(crate) fn net_mode(mode: crpg_sim::EncounterMode) -> EncounterMode {
    match mode {
        crpg_sim::EncounterMode::TurnBased => EncounterMode::TurnBased,
        crpg_sim::EncounterMode::RealTime => EncounterMode::RealTime,
    }
}

/// The runtime key of `entity` in `world` (Reading 8).
pub(crate) fn key_of(world: &World, entity: EntityId) -> EntityKey {
    EntityKey {
        area: world.area().unwrap_or(HOST_UNBOUND_AREA),
        entity,
    }
}

/// The encounter's values in authority terms, minus the timeline slice.
#[derive(Debug)]
struct EncounterShape {
    area: Ulid,
    mode: EncounterMode,
    pending_mode: Option<EncounterMode>,
    clock: Option<ClockFact>,
}

/// One session's owned read of `World` (C-2): everything the two views need
/// for one call, borrowed back as contexts.
#[derive(Debug)]
pub(crate) struct WorldRead {
    area: Ulid,
    tick: u64,
    facts: Vec<EntityFact>,
    rows: Vec<FactRow>,
    turn: Option<TurnState>,
    turn_granted: bool,
    encounter: Option<EncounterShape>,
    timeline: Vec<(i32, EntityKey)>,
    encounter_grants: EncounterGrants,
    despawned: Vec<EntityKey>,
}

impl WorldRead {
    /// The keys a publication page may read a removal of as `Despawned`
    /// (D-7); empty for a staging.
    pub(crate) fn with_despawned(mut self, despawned: Vec<EntityKey>) -> Self {
        self.despawned = despawned;
        self
    }

    /// The interest view's context for `epoch`.
    pub(crate) fn viewer(&self, epoch: [u8; 16]) -> ViewerContext<'_> {
        ViewerContext {
            epoch: SessionEpoch(epoch),
            tick: self.tick,
            interest: InterestSet { area: self.area },
            facts: &self.facts,
            despawned: &self.despawned,
            turn: self.turn,
            turn_granted: self.turn_granted,
            resend_turn: false,
        }
    }

    /// The fact view's context.
    pub(crate) fn fact_context(&self) -> FactContext<'_> {
        FactContext {
            rows: &self.rows,
            encounter: self.encounter.as_ref().map(|shape| EncounterInput {
                area: shape.area,
                mode: shape.mode,
                pending_mode: shape.pending_mode,
                clock: shape.clock,
                timeline: &self.timeline,
            }),
            grants: self.encounter_grants,
            resend: false,
        }
    }
}

/// The operator's fact grant for `entity`: all `false` when absent.
fn entity_grant(facts: &FactGrants, entity: EntityId) -> EntityFactGrant {
    facts
        .entities
        .binary_search_by_key(&entity, |grant| grant.entity)
        .ok()
        .map_or(
            EntityFactGrant {
                entity,
                pools: false,
                effects: false,
                stats: false,
                tags: false,
            },
            |index| facts.entities[index],
        )
}

/// One `PoolFact` of a resource pool. A macro, because this crate has no edge
/// to `crpg-rules` and so cannot name the pool type in a function signature.
macro_rules! pool_fact {
    ($pool:expr) => {{
        let pool = $pool;
        PoolFact {
            pool: pool.id().0,
            current: pool.current(),
            max: pool.max(),
        }
    }};
}

/// Reads `world` for one viewer with disclosure grants `grants`, fact grants
/// `facts` and control list `control` (C-2). Pure: it mints no id and keeps
/// nothing; the views mint and remember.
pub(crate) fn read_world(
    world: &World,
    tick: u64,
    grants: &DisclosureGrants,
    facts: &FactGrants,
    control: &[EntityId],
) -> WorldRead {
    let area = world.area().unwrap_or(HOST_UNBOUND_AREA);
    let mut entity_facts = Vec::new();
    let mut rows = Vec::new();
    for disclosure in &grants.entities {
        let entity = disclosure.entity;
        if !disclosure.present || !world.contains(entity) {
            continue;
        }
        let combatant = world.combatants().get(entity);
        entity_facts.push(EntityFact {
            key: key_of(world, entity),
            fields: FieldMask {
                presence: true,
                health: disclosure.health,
            },
            health: combatant.map(|state| HealthFact {
                health: state.health(),
                max_health: state.max_health(),
                dead: state.dead(),
            }),
            resend_health: false,
        });
        let grant = entity_grant(facts, entity);
        let key = key_of(world, entity);
        if let Some(state) = combatant {
            let mut pools = vec![pool_fact!(state.action_pool())];
            pools.extend(state.extra_pools().iter().map(|pool| pool_fact!(pool)));
            // `attached()` sorted by effect; the sort is stable.
            let mut attached = state.attached().to_vec();
            attached.sort_by_key(|effect| effect.effect);
            rows.push(FactRow {
                key,
                fields: FactMask {
                    pools: grant.pools,
                    effects: grant.effects,
                    order: control.binary_search(&entity).is_ok(),
                    stats: grant.stats,
                    tags: grant.tags,
                },
                pools,
                effects: attached
                    .iter()
                    .map(|effect| EffectFact {
                        effect: effect.effect,
                        expires_round: u64::from(effect.expires_round),
                    })
                    .collect(),
                order: state.order().map(|order| OrderInput {
                    ability: order.ability,
                    target: key_of(world, order.target),
                    repeat: order.repeat,
                }),
                stats: stat_facts(state.attributes(), state.fixed()),
                tags: tag_facts(state.tags()),
            });
        } else if let Some(creature) = world.creatures().get(entity) {
            rows.push(FactRow {
                key,
                fields: FactMask {
                    pools: grant.pools,
                    effects: false,
                    order: false,
                    stats: grant.stats,
                    tags: grant.tags,
                },
                pools: creature
                    .pools()
                    .iter()
                    .map(|pool| pool_fact!(pool))
                    .collect(),
                effects: Vec::new(),
                order: None,
                stats: stat_facts(creature.attributes(), creature.fixed()),
                tags: tag_facts(creature.tags()),
            });
        }
    }
    let combat = world.combat();
    WorldRead {
        area,
        tick,
        facts: entity_facts,
        rows,
        turn: combat.map(|combat| TurnState {
            area,
            active: combat.active,
            round: u64::from(combat.round),
        }),
        turn_granted: grants.turn_state,
        encounter: combat.map(|combat| EncounterShape {
            area,
            mode: net_mode(combat.mode),
            pending_mode: combat.pending_mode.map(net_mode),
            clock: combat.clock.map(|clock| ClockFact {
                round_ticks: clock.round_ticks,
                elapsed: clock.elapsed,
            }),
        }),
        timeline: world
            .timeline()
            .iter()
            .map(|(key, entity)| (key.0, key_of(world, entity)))
            .collect(),
        encounter_grants: facts.encounter,
        despawned: Vec::new(),
    }
}

/// Attributes then fixed stats, stored order (C-2).
fn stat_facts(
    attributes: &[(String, i32)],
    fixed: &[(String, crpg_core::Fx16_16)],
) -> Vec<StatFact> {
    attributes
        .iter()
        .map(|(name, value)| StatFact {
            name: name.clone(),
            value: StatValue::Int(*value),
        })
        .chain(fixed.iter().map(|(name, value)| StatFact {
            name: name.clone(),
            value: StatValue::Fixed(value.to_raw()),
        }))
        .collect()
}

/// Tags sorted by bytes and deduplicated (C-2).
fn tag_facts(tags: &[String]) -> Vec<String> {
    let mut tags = tags.to_vec();
    tags.sort();
    tags.dedup();
    tags
}
