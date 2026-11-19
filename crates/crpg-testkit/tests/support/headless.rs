//! Tests-only headless client support for T041a (`tasks/T041.md`, H1–H8):
//! content start, grants, a deterministic simulated network on the client
//! side of the link, a `HeadlessClient` speaking protocol v4 exactly as the
//! Godot bridge will, scripts that read only their own replica, the `Arena`
//! over `HostLoop<LocalHost>`, the independent mirror and `canon`/`world_view`.
//!
//! Each suite includes this module with `#[path = "support/headless.rs"] mod
//! headless;` and uses only a subset of it, so the first line allows dead
//! code as `crates/crpg-server/tests/support/quic_host_rig.rs:1` does. No
//! source, fixture or golden changes anywhere.

#![allow(dead_code)]

#[path = "combat.rs"]
#[allow(dead_code)]
mod combat;

use std::collections::BTreeMap;
use std::num::{NonZeroU32, NonZeroUsize};

use crpg_core::{DeterministicRng, EntityId, Fx16_16, Ulid};
use crpg_data::{
    Ability, Area, Creature, Document, Effect, Encounter, LengthWire, MovementWire, OutcomeTable,
    Placement, Ruleset, TurnMovementWire,
};
use crpg_nav::{bake_box, NavMesh, NavRect};
use crpg_net::codec_v4;
use crpg_net::facts_v3::EncounterGrants;
use crpg_net::movement_v4::{self, MoveBody, MoveDown, MoveReceipt, MOVEMENT_VERSION};
use crpg_net::outbox_v4::MoveOutbox;
use crpg_net::pool_v4::{
    GateMode, ReassemblyPoolV4, ReceiverIdV4, ReplicaUpdateV4, TransferError, TransferLimitsV3,
};
use crpg_net::protocol::{NetId, LANE_COMBAT};
use crpg_net::protocol_transfer::{TransferUp, TurnFact};
use crpg_net::protocol_v3::{
    DeltaOp as DeltaOpV3, EncounterFacts, HostFacts, PauseRights, ReceiptStatus as ReceiptV3,
    TimelineEntry,
};
use crpg_net::protocol_v4::{
    DeltaOp, IntentBody, IntentFrame, Point, SessionEpoch, PROTOCOL_VERSION as V4,
};
use crpg_net::replica::ReplicaError;
use crpg_net::replica_v3::{PauseState, ReplicaEntityV3};
use crpg_net::replica_v4::ReplicaWorldV4;
use crpg_net::transfer_v3::{self, TRANSFER_VERSION_V3};
use crpg_net::transport::Transport;
use crpg_server::host::{
    ControlGrant, DisclosureGrants, EntityDisclosure, EntityFactGrant, FactGrants, Host,
    HostConfig, PeerHandle, ProtocolSelection, DEFAULT_HOST_GRANTS,
};
use crpg_server::run::{HostLoop, LocalClient, LocalHost};
use crpg_sim::{
    history_hash, CombatAction, EncounterMode, EncounterSpec, HistoryWorld, MoveRoute, NavRevision,
    PlacementAndArea, RouteTag,
};

/// The `HistoryWorld` seed of every run (T037e's).
pub const WORLD_SEED: u64 = 11;
/// The simulated network's default master seed.
pub const NET_SEED: u64 = 0x7041_0000_0000_0001;
/// `LocalHost::attach` capacity of every link.
pub const LINK_CAPACITY: usize = 256;
/// Cycles a settle runs (Reading 7).
pub const SETTLE_CYCLES: u64 = 60;
/// Consecutive losses after which a reliable frame always gets through.
pub const MAX_LOSSES: u32 = 8;
/// Milliseconds per cycle: `crpg_server::run::TICK_PERIOD_MS`.
pub const PERIOD_MS: u64 = 50;

// `K`, `N`, `P` and the two placements of `combat_basic`.

/// `ulid("…")`: an authored identity out of the campaign's ULID text.
fn ulid(text: &str) -> Ulid {
    text.parse().expect("a canonical ULID")
}

/// The strike ability `"0000000000000000000000000K"`.
pub fn strike() -> Ulid {
    ulid("0000000000000000000000000K")
}

/// The encounter `first-blood` `"0000000000000000000000000N"`.
pub fn first_blood() -> Ulid {
    ulid("0000000000000000000000000N")
}

/// The primary pool `"0000000000000000000000000P"`.
pub fn pool() -> Ulid {
    ulid("0000000000000000000000000P")
}

/// The hero fixture placement `"0000000000000000000000000E"`.
fn hero_placement() -> Ulid {
    ulid("0000000000000000000000000E")
}

/// The goblin fixture placement `"0000000000000000000000000F"`.
fn goblin_placement() -> Ulid {
    ulid("0000000000000000000000000F")
}

/// `NetId::new(raw)`.
pub fn n(raw: u64) -> NetId {
    NetId::new(raw).expect("nonzero")
}

/// A raw wire point `[x, y, z]`.
pub fn pt(x: i32, y: i32, z: i32) -> Point {
    Point::new([x, y, z]).expect("within the wire bound")
}

fn rect_raw(min_x: i32, min_z: i32, max_x: i32, max_z: i32) -> NavRect {
    NavRect {
        min_x: Fx16_16::from_raw(min_x),
        min_z: Fx16_16::from_raw(min_z),
        max_x: Fx16_16::from_raw(max_x),
        max_z: Fx16_16::from_raw(max_z),
    }
}

/// The mesh `bind_area_bounds` bakes from the area's two bounds.
fn area_mesh(min: [Fx16_16; 3], max: [Fx16_16; 3]) -> NavMesh {
    bake_box(
        min[1],
        rect_raw(
            min[0].to_raw(),
            min[2].to_raw(),
            max[0].to_raw(),
            max[2].to_raw(),
        ),
        &[],
    )
    .expect("the authored bounds bake")
}

// ---------------------------------------------------------------------------
// Content and the start.
// ---------------------------------------------------------------------------

/// One loaded campaign document of each kind the spec consumes.
struct Content {
    encounter: Encounter,
    ruleset: Ruleset,
    area: Area,
    abilities: BTreeMap<Ulid, Ability>,
    outcome_tables: BTreeMap<Ulid, OutcomeTable>,
    placements: BTreeMap<Ulid, (Placement, Ulid)>,
    creatures: BTreeMap<Ulid, Creature>,
    effects: BTreeMap<Ulid, Effect>,
}

impl Content {
    /// Loads `combat_basic` through the T016c support loader.
    fn load() -> Self {
        let content = combat::CombatContent::load().expect("combat_basic loads");
        let mut encounter: Option<Encounter> = None;
        let mut ruleset: Option<Ruleset> = None;
        let mut area: Option<Area> = None;
        let mut abilities = BTreeMap::new();
        let mut outcome_tables = BTreeMap::new();
        let mut placements = BTreeMap::new();
        let mut creatures = BTreeMap::new();
        let mut effects = BTreeMap::new();
        for doc in content.loaded.documents.values() {
            match doc {
                Document::Encounter(value) => encounter = Some(value.clone()),
                Document::Ruleset(value) => ruleset = Some(value.clone()),
                Document::Area(value) => area = Some(value.clone()),
                Document::Ability(value) => {
                    abilities.insert(value.id, value.clone());
                }
                Document::OutcomeTable(value) => {
                    outcome_tables.insert(value.id, value.clone());
                }
                Document::Placements(value) => {
                    for placement in &value.placements {
                        placements.insert(placement.id, (placement.clone(), value.area));
                    }
                }
                Document::Creature(value) => {
                    creatures.insert(value.id, value.clone());
                }
                Document::Effect(value) => {
                    effects.insert(value.id, value.clone());
                }
                _ => {}
            }
        }
        Self {
            encounter: encounter.expect("the campaign has first-blood"),
            ruleset: ruleset.expect("the campaign has its ruleset"),
            area: area.expect("the campaign has its start area"),
            abilities,
            outcome_tables,
            placements,
            creatures,
            effects,
        }
    }

    /// The ruleset with T037e's in-memory movement block (`round_ticks` too
    /// for the real-time case).
    fn moved_ruleset(&self, round_ticks: Option<u32>) -> Ruleset {
        let mut ruleset = self.ruleset.clone();
        ruleset.movement = Some(Box::new(MovementWire {
            speed: LengthWire::Constant(Fx16_16::from_raw(65536)),
            measure: None,
            turn: TurnMovementWire {
                pool: pool(),
                step: LengthWire::Constant(Fx16_16::from_raw(65536)),
                step_cost: 1,
            },
        }));
        ruleset.round_ticks = round_ticks;
        ruleset
    }
}

/// A started `first-blood` with the area's mesh bound, and its mirror.
pub struct Start {
    /// The host wrapping the started history.
    pub host: Host,
    /// The independent mirror authority (a clone of the started history).
    pub mirror: HistoryWorld,
    /// The hero combatant (placement `0E`).
    pub hero: EntityId,
    /// The goblin combatant (placement `0F`).
    pub goblin: EntityId,
    /// The mesh the area's bounds bake.
    pub mesh: NavMesh,
    /// The revision of that mesh.
    pub revision: NavRevision,
}

/// Loads `combat_basic`, starts `first-blood` in `mode` under the authored
/// movement rules, wraps a clone in the host and the mirror, binds the area
/// bounds, and asserts the two authorities agree.
pub fn start(mode: EncounterMode, round_ticks: Option<u32>) -> Start {
    let content = Content::load();
    let ruleset = content.moved_ruleset(round_ticks);
    let abilities: BTreeMap<Ulid, &Ability> = content
        .abilities
        .iter()
        .map(|(id, value)| (*id, value))
        .collect();
    let outcome_tables: BTreeMap<Ulid, &OutcomeTable> = content
        .outcome_tables
        .iter()
        .map(|(id, value)| (*id, value))
        .collect();
    let placements: BTreeMap<Ulid, PlacementAndArea<'_>> = content
        .placements
        .iter()
        .map(|(id, (placement, area))| {
            (
                *id,
                PlacementAndArea {
                    placement,
                    area: *area,
                },
            )
        })
        .collect();
    let creatures: BTreeMap<Ulid, &Creature> = content
        .creatures
        .iter()
        .map(|(id, value)| (*id, value))
        .collect();
    let effects: BTreeMap<Ulid, &Effect> = content
        .effects
        .iter()
        .map(|(id, value)| (*id, value))
        .collect();
    let spec = EncounterSpec {
        encounter: &content.encounter,
        ruleset: &ruleset,
        abilities: &abilities,
        outcome_tables: &outcome_tables,
        placements: &placements,
        creatures: &creatures,
        effects: &effects,
    };
    let mut history = HistoryWorld::new_in_area(WORLD_SEED, content.area.id);
    history
        .start_encounter_in_mode(&spec, mode)
        .expect("first-blood starts");
    history
        .acknowledge(history.last_sequence())
        .expect("acknowledges its own start range");
    let bound: Vec<(EntityId, Ulid)> = history
        .world()
        .combatants()
        .iter()
        .map(|(entity, combatant)| (entity, combatant.placement()))
        .collect();
    assert_eq!(bound.len(), 2, "two authored participants");
    // Authored order: the hero's placement is 0E, the goblin's is 0F.
    assert_eq!(bound[0].1, hero_placement());
    assert_eq!(bound[1].1, goblin_placement());
    let hero = bound[0].0;
    let goblin = bound[1].0;
    let mut mirror = history.clone();
    let bounds = (content.area.bounds.min, content.area.bounds.max);
    let mesh = area_mesh(bounds.0, bounds.1);
    let mut host = Host::from_history(
        history,
        HostConfig {
            protocol: ProtocolSelection::V2,
            incarnation: 1,
        },
        0,
    )
    .expect("wraps the started history");
    let binding = host
        .bind_area_bounds(bounds.0, bounds.1)
        .expect("the authored bounds bind");
    mirror
        .bind_nav(binding.revision)
        .expect("the mirror binds the revision");
    mirror
        .acknowledge(mirror.last_sequence())
        .expect("acknowledges its own range");
    assert_eq!(host.authority_hash(), history_hash(&mirror));
    Start {
        host,
        mirror,
        hero,
        goblin,
        mesh,
        revision: binding.revision,
    }
}

// ---------------------------------------------------------------------------
// Grants.
// ---------------------------------------------------------------------------

/// Which grant set a seat gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grants {
    /// Every disclosure and fact flag, the strike and the encounter.
    Full,
    /// Present and health only, no abilities or encounter facts.
    Observer,
}

fn full_disclosure(entity: EntityId) -> EntityDisclosure {
    EntityDisclosure {
        entity,
        present: true,
        health: true,
        spawn: true,
        despawn: true,
        died: true,
        action_actor: true,
        action_target: true,
        action_outcome: true,
        action_damage: true,
        turn_start: true,
    }
}

fn observer_disclosure(entity: EntityId) -> EntityDisclosure {
    EntityDisclosure {
        entity,
        present: true,
        health: true,
        spawn: false,
        despawn: false,
        died: false,
        action_actor: false,
        action_target: false,
        action_outcome: false,
        action_damage: false,
        turn_start: false,
    }
}

fn all_fact_flags(entity: EntityId) -> EntityFactGrant {
    EntityFactGrant {
        entity,
        pools: true,
        effects: true,
        stats: true,
        tags: true,
    }
}

/// The disclosure grants for `grants` over the started `hero` and `goblin`.
pub fn disclosure(grants: Grants, start: &Start) -> DisclosureGrants {
    match grants {
        Grants::Full => DisclosureGrants {
            entities: vec![full_disclosure(start.hero), full_disclosure(start.goblin)],
            abilities: vec![strike()],
            encounters: vec![first_blood()],
            turn_round: true,
            encounter_end: true,
            turn_state: true,
        },
        Grants::Observer => DisclosureGrants {
            entities: vec![
                observer_disclosure(start.hero),
                observer_disclosure(start.goblin),
            ],
            abilities: Vec::new(),
            encounters: Vec::new(),
            turn_round: false,
            encounter_end: false,
            turn_state: true,
        },
    }
}

/// The fact grants for `grants` over the started `hero` and `goblin`.
pub fn fact_grants(grants: Grants, start: &Start) -> FactGrants {
    match grants {
        Grants::Full => FactGrants {
            entities: vec![all_fact_flags(start.hero), all_fact_flags(start.goblin)],
            encounter: EncounterGrants {
                mode: true,
                pending_mode: true,
                clock: true,
                timeline: true,
            },
            host: DEFAULT_HOST_GRANTS,
        },
        Grants::Observer => FactGrants::default(),
    }
}

/// Every seat's pause rights: none.
pub const NO_PAUSE: PauseRights = PauseRights {
    pause: false,
    unpause: false,
};

// ---------------------------------------------------------------------------
// The simulated network.
// ---------------------------------------------------------------------------

/// One channel's fault profile (Reading 5). Times are in cycles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Faults {
    /// Percent of reliable attempts or datagram frames lost (0..=100).
    pub loss_percent: u32,
    /// Percent of datagram frames duplicated (0..=100).
    pub dup_percent: u32,
    /// Fixed latency added to every frame, in cycles.
    pub latency: u64,
    /// Uniform jitter `0..=jitter`, in cycles.
    pub jitter: u64,
    /// One reliable retransmission timeout, in cycles.
    pub rto: u64,
}

impl Faults {
    /// A lossless, immediate, ordered channel.
    pub const CLEAN: Faults = Faults {
        loss_percent: 0,
        dup_percent: 0,
        latency: 0,
        jitter: 0,
        rto: 1,
    };
    /// The spec's numbers: 3 % loss, 2-cycle latency, 0–4 jitter, 4-cycle rto,
    /// 1 % duplication.
    pub const SPEC: Faults = Faults {
        loss_percent: 3,
        dup_percent: 1,
        latency: 2,
        jitter: 4,
        rto: 4,
    };
}

/// Whether a channel is ordered/reliable or a lossy datagram.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelKind {
    /// Every frame arrives once, in order, after latency plus jitter.
    Reliable,
    /// Frames may be lost, duplicated and reordered.
    Datagram,
}

/// What a channel's faults did, counted since `new`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChannelStats {
    /// Calls to `send`.
    pub sent: u64,
    /// Reliable: lost attempts; datagram: frames dropped.
    pub lost: u64,
    /// Datagram copies added.
    pub duplicated: u64,
    /// `drop_next` discards.
    pub discarded: u64,
}

/// One direction of one kind of traffic (B-1, B-2).
pub struct Channel {
    kind: ChannelKind,
    rng: DeterministicRng,
    name: String,
    queue: BTreeMap<(u64, u64), Vec<u8>>,
    last_due: u64,
    next_order: u64,
    drop_next: bool,
    stats: ChannelStats,
}

impl Channel {
    /// A channel of `kind` drawing from `DeterministicRng::from_seed(seed)`
    /// on the named stream.
    pub fn new(kind: ChannelKind, seed: u64, name: &str) -> Channel {
        Channel {
            kind,
            rng: DeterministicRng::from_seed(seed),
            name: name.to_string(),
            queue: BTreeMap::new(),
            last_due: 0,
            next_order: 0,
            drop_next: false,
            stats: ChannelStats::default(),
        }
    }

    /// `d(k)`: one draw from the channel's own stream, uniform in `[0, k)`.
    fn draw(&mut self, k: u32) -> u32 {
        let name = self.name.clone();
        self.rng
            .stream(&name)
            .gen_range_u32(NonZeroU32::new(k).expect("a nonzero bound"))
    }

    /// Queues `bytes` under `faults`, drawing exactly as B-1/B-2 specify.
    pub fn send(&mut self, now: u64, bytes: Vec<u8>, faults: Faults) {
        if self.drop_next {
            self.drop_next = false;
            self.stats.discarded += 1;
            return;
        }
        self.stats.sent += 1;
        match self.kind {
            ChannelKind::Reliable => {
                let mut losses = 0u32;
                while losses < MAX_LOSSES && self.draw(100) < faults.loss_percent {
                    losses += 1;
                }
                let j = self.draw((faults.jitter + 1) as u32);
                let due = (now + u64::from(losses) * faults.rto + faults.latency + u64::from(j))
                    .max(self.last_due);
                self.last_due = due;
                self.stats.lost += u64::from(losses);
                let order = self.next_order;
                self.next_order += 1;
                self.queue.insert((due, order), bytes);
            }
            ChannelKind::Datagram => {
                if self.draw(100) < faults.loss_percent {
                    self.stats.lost += 1;
                    return;
                }
                let due = now + faults.latency + u64::from(self.draw((faults.jitter + 1) as u32));
                let order = self.next_order;
                self.next_order += 1;
                self.queue.insert((due, order), bytes.clone());
                if self.draw(100) < faults.dup_percent {
                    let due2 =
                        now + faults.latency + u64::from(self.draw((faults.jitter + 1) as u32));
                    let order2 = self.next_order;
                    self.next_order += 1;
                    self.queue.insert((due2, order2), bytes);
                    self.stats.duplicated += 1;
                }
            }
        }
    }

    /// Removes and returns every frame with `due <= now`, in ascending
    /// `(due, order)`.
    pub fn take_due(&mut self, now: u64) -> Vec<Vec<u8>> {
        let keys: Vec<(u64, u64)> = self
            .queue
            .keys()
            .copied()
            .take_while(|(due, _)| *due <= now)
            .collect();
        let mut out = Vec::with_capacity(keys.len());
        for key in keys {
            out.push(self.queue.remove(&key).expect("the key was listed"));
        }
        out
    }

    /// The number of frames still queued.
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// Whether no frame is queued.
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// The next `send` is discarded whatever its kind (D3's broken network).
    pub fn drop_next(&mut self) {
        self.drop_next = true;
    }

    /// What the channel's faults did since `new`.
    pub fn stats(&self) -> ChannelStats {
        self.stats
    }
}

// ---------------------------------------------------------------------------
// Scripts and the client.
// ---------------------------------------------------------------------------

/// Which of a client's four channels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Which {
    /// The client-to-host intent channel.
    Up,
    /// The host-to-client lane-0 channel.
    Down0,
    /// The host-to-client lane-1 receipt channel.
    Down1,
    /// The host-to-client sample datagram channel.
    Datagram,
}

/// What a client does each cycle (B-6 to B-10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Script {
    /// Do nothing.
    Idle,
    /// Move `actor` one step back and forth between `points` and end its turn.
    Patrol {
        /// The controlled combatant.
        actor: NetId,
        /// The two destinations, alternating.
        points: [Point; 2],
    },
    /// Close to `first_move`, then strike `target` until it dies.
    Fight {
        /// The controlled combatant.
        actor: NetId,
        /// The target to strike.
        target: NetId,
        /// The first move's destination.
        first_move: Point,
    },
    /// Send `EndTurn` for a combatant it does not control, every `every`
    /// cycles.
    Intruder {
        /// The foreign combatant.
        actor: NetId,
        /// The cadence, in cycles.
        every: u64,
    },
    /// Walk `actor` back and forth between `points` on lane 1.
    Walk {
        /// The controlled combatant.
        actor: NetId,
        /// The two destinations, alternating.
        points: [Point; 2],
    },
}

/// One lane-0 intent the arena handed to a link, in admission order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Admitted {
    /// The client's index in the arena's `clients` vector.
    pub client: usize,
    /// The client's peer handle.
    pub peer: PeerHandle,
    /// The decoded intent frame.
    pub frame: IntentFrame,
}

/// One scripted headless client (Reading 3).
pub struct HeadlessClient {
    /// The seat name, or `"{name}~"` for a twin.
    pub name: String,
    /// The bound peer handle.
    pub peer: PeerHandle,
    /// The session epoch.
    pub epoch: [u8; 16],
    /// Set on a twin: its client's index.
    pub twin_of: Option<usize>,
    /// The client's filtered replica.
    pub replica: ReplicaWorldV4,
    /// The lane-1 sender state.
    pub outbox: MoveOutbox,
    /// This client's fault profile.
    pub faults: Faults,
    /// What this client does each cycle.
    pub script: Script,
    /// Whether the script is paused (during a settle).
    pub paused: bool,
    /// Every lane-0 intent sent, in send order.
    pub sent: Vec<IntentFrame>,
    /// Every lane-0 receipt seen, by seq, in arrival order.
    pub receipts: BTreeMap<u64, Vec<ReceiptV3>>,
    /// `(seq, cycle)` of every lane-0 receipt, in arrival order.
    pub receipt_cycles: Vec<(u64, u64)>,
    /// Every lane-1 receipt, in arrival order.
    pub move_receipts: Vec<MoveReceipt>,
    /// Refusals by the pool, recorded instead of panicking (B-5).
    pub gate_errors: Vec<TransferError>,
    /// Refusals by the replica, recorded instead of panicking (B-5).
    pub replica_errors: Vec<ReplicaError>,

    link: LocalClient,
    pool: ReassemblyPoolV4,
    rid: ReceiverIdV4,
    up: Channel,
    down0: Channel,
    down1: Channel,
    datagram: Channel,
    next_seq: u64,
    acted: Option<TurnFact>,
    awaiting: Option<u64>,
    moves_applied: u64,
    walk_leg: u64,
    saved_faults: Faults,
}

impl HeadlessClient {
    /// One of the client's four channels, mutably.
    pub fn channel_mut(&mut self, which: Which) -> &mut Channel {
        match which {
            Which::Up => &mut self.up,
            Which::Down0 => &mut self.down0,
            Which::Down1 => &mut self.down1,
            Which::Datagram => &mut self.datagram,
        }
    }

    /// Frames in all four channels.
    pub fn in_flight(&self) -> usize {
        self.up.len() + self.down0.len() + self.down1.len() + self.datagram.len()
    }

    /// `ReassemblyPoolV4::held_delta_bytes`.
    pub fn held_delta_bytes(&self) -> usize {
        self.pool.held_delta_bytes()
    }

    /// Seqs of the client's own `DeclareMove`s whose receipt was `Applied`.
    pub fn moves_applied(&self) -> u64 {
        self.moves_applied
    }

    /// Runs the script against the replica as it stood after cycle `c - 1`.
    fn run_script(&mut self, c: u64) {
        if self.paused {
            return;
        }
        match self.script {
            Script::Idle => {}
            Script::Patrol { actor, points } => self.patrol(actor, points, c),
            Script::Fight {
                actor,
                target,
                first_move,
            } => self.fight(actor, target, first_move, c),
            Script::Intruder { actor, every } => self.intrude(actor, every, c),
            Script::Walk { actor, points } => self.walk(actor, points, c),
        }
    }

    /// B-8: move one step, then end the turn once the move's receipt arrives.
    fn patrol(&mut self, actor: NetId, points: [Point; 2], c: u64) {
        let t = self.replica.v3().turn().cloned();
        if let Some(s) = self.awaiting {
            if self.receipts.contains_key(&s) {
                self.awaiting = None;
                if t == self.acted && t.is_some_and(|f| f.active == Some(actor)) {
                    self.send_lane0(actor, IntentBody::EndTurn, c);
                }
            }
            return;
        }
        if let Some(f) = t {
            if f.active == Some(actor) && self.acted.as_ref() != Some(&f) {
                self.acted = Some(f);
                let destination = points[(self.moves_applied % 2) as usize];
                let seq = self.send_lane0(actor, IntentBody::DeclareMove { destination }, c);
                self.awaiting = Some(seq);
            }
        }
    }

    /// B-9: as `Patrol`, but stop once anyone is dead and strike after the
    /// first move.
    fn fight(&mut self, actor: NetId, target: NetId, first_move: Point, c: u64) {
        if self
            .replica
            .v3()
            .entities()
            .iter()
            .any(|row| row.health.as_ref().is_some_and(|h| h.dead))
        {
            return;
        }
        let t = self.replica.v3().turn().cloned();
        if let Some(s) = self.awaiting {
            if self.receipts.contains_key(&s) {
                self.awaiting = None;
                if t == self.acted && t.is_some_and(|f| f.active == Some(actor)) {
                    self.send_lane0(actor, IntentBody::EndTurn, c);
                }
            }
            return;
        }
        if let Some(f) = t {
            if f.active == Some(actor) && self.acted.as_ref() != Some(&f) {
                self.acted = Some(f);
                let never_moved = !self
                    .sent
                    .iter()
                    .any(|fr| matches!(fr.body, IntentBody::DeclareMove { .. }));
                let body = if never_moved {
                    IntentBody::DeclareMove {
                        destination: first_move,
                    }
                } else {
                    IntentBody::DeclareAction {
                        ability: strike(),
                        target,
                    }
                };
                let seq = self.send_lane0(actor, body, c);
                self.awaiting = Some(seq);
            }
        }
    }

    /// B-10: an `EndTurn` for a combatant the client does not control.
    fn intrude(&mut self, actor: NetId, every: u64, c: u64) {
        if c > 0 && c.is_multiple_of(every) {
            self.send_lane0(actor, IntentBody::EndTurn, c);
        }
    }

    /// B-11: walk on lane 1, one `MoveTo` per arrival.
    fn walk(&mut self, actor: NetId, points: [Point; 2], c: u64) {
        if self.replica.position(actor).is_none() {
            return;
        }
        if self.walk_leg == 0 {
            self.send_move(
                actor,
                MoveBody::MoveTo {
                    destination: points[0],
                },
                c,
            );
            self.walk_leg = 1;
            return;
        }
        if self.outbox.pending().is_empty() {
            if let Some(p) = self.replica.position(actor) {
                if p.point == points[((self.walk_leg - 1) % 2) as usize] && !p.moving {
                    let destination = points[(self.walk_leg % 2) as usize];
                    self.send_move(actor, MoveBody::MoveTo { destination }, c);
                    self.walk_leg += 1;
                }
            }
        }
    }

    /// B-13: send one lane-0 `body` for `actor`, returning its seq.
    fn send_lane0(&mut self, actor: NetId, body: IntentBody, c: u64) -> u64 {
        let frame = IntentFrame {
            epoch: self.epoch,
            lane: LANE_COMBAT,
            seq: self.next_seq,
            observed_tick: self.replica.v3().server_tick(),
            actor,
            body,
        };
        let bytes = codec_v4::encode_intent(&frame).expect("encode");
        self.up.send(c, bytes, self.faults);
        self.sent.push(frame);
        self.next_seq += 1;
        frame.seq
    }

    /// A lane-1 send through the outbox and the `up` channel.
    fn send_move(&mut self, actor: NetId, body: MoveBody, c: u64) {
        let intent = self.outbox.send(actor, body).expect("outbox");
        let bytes = movement_v4::encode_intent(&intent).expect("encodes");
        self.up.send(c, bytes, self.faults);
    }

    /// B-6 step 5: route one host frame to the right down channel.
    fn route_down(&mut self, bytes: &[u8], c: u64) {
        match bytes.first() {
            Some(&TRANSFER_VERSION_V3) | Some(&V4) => {
                self.down0.send(c, bytes.to_vec(), self.faults);
            }
            Some(&MOVEMENT_VERSION) => match movement_v4::decode_down(bytes) {
                Ok(MoveDown::Receipt(_)) => self.down1.send(c, bytes.to_vec(), self.faults),
                Ok(MoveDown::Sample(_)) => self.datagram.send(c, bytes.to_vec(), self.faults),
                _ => panic!("down frame"),
            },
            _ => panic!("down frame"),
        }
    }

    /// B-6 step 6: drain the down channels into the pool, replica and outbox.
    fn deliver(&mut self, c: u64) {
        let now = c * PERIOD_MS;
        for bytes in self.down0.take_due(c) {
            let result: Result<(), TransferError> = match bytes.first() {
                Some(&TRANSFER_VERSION_V3) => {
                    self.pool.offer_transfer(self.rid, &bytes, now).map(|_| ())
                }
                Some(&V4) => self.pool.offer_delta(self.rid, &bytes, now).map(|_| ()),
                _ => panic!("down frame"),
            };
            if let Err(e) = result {
                self.gate_errors.push(e);
            }
        }
        while let Some(update) = self.pool.pop_update(self.rid) {
            match update {
                ReplicaUpdateV4::Snapshot(p) => match self.replica.apply_snapshot(&p) {
                    Ok(()) => {
                        let bytes = transfer_v3::encode_up(&TransferUp::Published {
                            epoch: self.epoch,
                            transfer: p.transfer,
                        })
                        .expect("encodes");
                        self.up.send(c, bytes, self.faults);
                    }
                    Err(e) => self.replica_errors.push(e),
                },
                ReplicaUpdateV4::Delta(f) => match self.replica.apply_delta(&f) {
                    Ok(()) => {
                        for op in &f.ops {
                            if let DeltaOp::V3(DeltaOpV3::Receipt { seq, status, .. }) = op {
                                self.receipts.entry(*seq).or_default().push(*status);
                                self.receipt_cycles.push((*seq, c));
                                if *status == ReceiptV3::Applied
                                    && self.sent.iter().any(|fr| {
                                        fr.seq == *seq
                                            && matches!(fr.body, IntentBody::DeclareMove { .. })
                                    })
                                {
                                    self.moves_applied += 1;
                                }
                            }
                        }
                    }
                    Err(e) => self.replica_errors.push(e),
                },
            }
        }
        for bytes in self.down1.take_due(c) {
            let r = match movement_v4::decode_down(&bytes) {
                Ok(MoveDown::Receipt(r)) => r,
                _ => panic!("lane-1 receipt"),
            };
            self.outbox.receive(&r).expect("lane-1 receipt");
            self.move_receipts.push(r);
        }
        for bytes in self.datagram.take_due(c) {
            let s = match movement_v4::decode_down(&bytes) {
                Ok(MoveDown::Sample(s)) => s,
                _ => panic!("down frame"),
            };
            self.replica.apply_sample(&s);
        }
    }
}

// ---------------------------------------------------------------------------
// Comparison (B-14).
// ---------------------------------------------------------------------------

/// The comparable state of one replica (Reading 7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Canon {
    /// Every live row, ascending.
    pub entities: Vec<ReplicaEntityV3>,
    /// The last turn fact.
    pub turn: Option<TurnFact>,
    /// The encounter facts.
    pub encounter: EncounterFacts,
    /// The timeline image.
    pub timeline: Option<Vec<TimelineEntry>>,
    /// The pause state.
    pub pause: Option<PauseState>,
    /// The viewer's host facts.
    pub host_facts: Option<HostFacts>,
    /// Every disclosed position as `(point, moving)`.
    pub positions: BTreeMap<NetId, (Point, bool)>,
}

/// The comparable state of `replica`, leaving out the ticks and event
/// sequences that legitimately differ between sessions.
pub fn canon(replica: &ReplicaWorldV4) -> Canon {
    Canon {
        entities: replica.v3().entities().to_vec(),
        turn: replica.v3().turn().cloned(),
        encounter: *replica.v3().encounter(),
        timeline: replica.v3().timeline().map(<[_]>::to_vec),
        pause: replica.v3().pause(),
        host_facts: replica.v3().host_facts().cloned(),
        positions: replica
            .positions()
            .iter()
            .map(|(id, state)| (*id, (state.point, state.moving)))
            .collect(),
    }
}

/// `canon(replica)` with the per-viewer standing orders and host facts
/// removed (Reading 9).
pub fn world_view(replica: &ReplicaWorldV4) -> Canon {
    let mut c = canon(replica);
    for row in &mut c.entities {
        row.facts.order = None;
    }
    c.host_facts = None;
    c
}

// ---------------------------------------------------------------------------
// The mirror authority (B-12).
// ---------------------------------------------------------------------------

/// The mirror authority (Reading 8).
pub struct Mirror {
    /// The independently driven authority.
    pub world: HistoryWorld,
    mesh: NavMesh,
    revision: NavRevision,
    entity_of: BTreeMap<NetId, EntityId>,
    at: BTreeMap<EntityId, [Fx16_16; 3]>,
}

impl Mirror {
    /// Applies one admitted intent to the mirror, then acknowledges.
    fn apply(&mut self, admitted: &Admitted) {
        let actor = *self.entity_of.get(&admitted.frame.actor).expect("mirror");
        match &admitted.frame.body {
            IntentBody::EndTurn => {
                self.world
                    .perform_action(&CombatAction::EndTurn { actor })
                    .expect("mirror");
            }
            IntentBody::DeclareAction { ability, target } => {
                let target = *self.entity_of.get(target).expect("mirror");
                self.world
                    .perform_action(&CombatAction::UseAbility {
                        actor,
                        ability: *ability,
                        target,
                    })
                    .expect("mirror");
            }
            IntentBody::DeclareMove { destination } => {
                let to = destination.raw().map(Fx16_16::from_raw);
                let from = *self.at.get(&actor).expect("mirror");
                let points = self.mesh.find_path(from, to).expect("mirror").into_points();
                let route = MoveRoute {
                    revision: self.revision,
                    tag: RouteTag(admitted.frame.seq),
                    points,
                };
                self.world.perform_move(actor, &route).expect("mirror");
                self.at.insert(actor, to);
            }
            _ => panic!("mirror"),
        }
        self.world
            .acknowledge(self.world.last_sequence())
            .expect("mirror");
    }

    /// One tick, then the acknowledgement.
    fn tick(&mut self) {
        self.world.tick_with_movement().expect("the mirror ticks");
        self.world
            .acknowledge(self.world.last_sequence())
            .expect("mirror");
    }
}

// ---------------------------------------------------------------------------
// The arena (B-5, B-6, B-7).
// ---------------------------------------------------------------------------

/// One seat of an arena: a client and, when `twin`, its twin (Reading 6).
pub struct Seat {
    /// The seat's name; the twin is named `"{name}~"`.
    pub name: &'static str,
    /// The combatants the seat controls.
    pub control: Vec<EntityId>,
    /// The seat's grant set.
    pub grants: Grants,
    /// The client's fault profile.
    pub faults: Faults,
    /// The client's script.
    pub script: Script,
    /// Whether a faultless twin is bound right after the client.
    pub twin: bool,
}

/// One host loop, its seats' clients and twins, and the mirror.
pub struct Arena {
    /// The embedded host's run loop.
    pub host_loop: HostLoop<LocalHost>,
    /// Every client, ascending peer handle (each twin right after its client).
    pub clients: Vec<HeadlessClient>,
    /// The independent mirror, when one is set.
    pub mirror: Option<Mirror>,
    /// The hero combatant.
    pub hero: EntityId,
    /// The goblin combatant.
    pub goblin: EntityId,
    /// The next cycle to run.
    pub cycle: u64,
}

impl Arena {
    /// Starts `first-blood` in `mode`, binds every seat's client and twin,
    /// attaches every session and, when `mirror`, drives the independent
    /// authority.
    pub fn new(
        mode: EncounterMode,
        round_ticks: Option<u32>,
        seats: Vec<Seat>,
        net_seed: u64,
        mirror: bool,
    ) -> Arena {
        let mut start = start(mode, round_ticks);
        let hero = start.hero;
        let goblin = start.goblin;

        // Bind every seat's client and, when `twin`, its twin, in seat order.
        struct Bound {
            name: &'static str,
            script: Script,
            faults: Faults,
            peer: PeerHandle,
            twin: Option<PeerHandle>,
        }
        let mut bound: Vec<Bound> = Vec::new();
        for seat in seats {
            let control = ControlGrant {
                actors: seat.control,
            };
            let grants = disclosure(seat.grants, &start);
            let facts = fact_grants(seat.grants, &start);
            let peer = start
                .host
                .bind_v4_peer(control.clone(), grants.clone(), facts.clone(), NO_PAUSE, 0)
                .expect("binds a v4 peer");
            let twin = if seat.twin {
                Some(
                    start
                        .host
                        .bind_v4_peer(control, grants, facts, NO_PAUSE, 0)
                        .expect("binds the twin"),
                )
            } else {
                None
            };
            bound.push(Bound {
                name: seat.name,
                script: seat.script,
                faults: seat.faults,
                peer,
                twin,
            });
        }
        // Every session must map hero to n(1) and goblin to n(2).
        for b in &bound {
            assert_eq!(
                start.host.replica_id(b.peer, hero),
                Ok(Some(n(1))),
                "net ids"
            );
            assert_eq!(
                start.host.replica_id(b.peer, goblin),
                Ok(Some(n(2))),
                "net ids"
            );
            if let Some(twin) = b.twin {
                assert_eq!(start.host.replica_id(twin, hero), Ok(Some(n(1))), "net ids");
                assert_eq!(
                    start.host.replica_id(twin, goblin),
                    Ok(Some(n(2))),
                    "net ids"
                );
            }
        }

        let host = start.host;
        let mut host_loop = HostLoop::new(LocalHost::new(host), 0);

        let mut clients: Vec<HeadlessClient> = Vec::new();
        for b in bound {
            let client_index = clients.len();
            let epoch = host_loop.hosting().host().epoch(b.peer).expect("bound");
            let link = host_loop
                .hosting_mut()
                .attach(b.peer, NonZeroUsize::new(LINK_CAPACITY).expect("nonzero"))
                .expect("attaches");
            clients.push(new_client(
                b.name, b.peer, epoch, None, b.faults, b.script, link, net_seed,
            ));
            if let Some(twin) = b.twin {
                let epoch = host_loop.hosting().host().epoch(twin).expect("bound");
                let link = host_loop
                    .hosting_mut()
                    .attach(twin, NonZeroUsize::new(LINK_CAPACITY).expect("nonzero"))
                    .expect("attaches");
                let name = format!("{}~", b.name);
                clients.push(new_client(
                    &name,
                    twin,
                    epoch,
                    Some(client_index),
                    Faults::CLEAN,
                    Script::Idle,
                    link,
                    net_seed,
                ));
            }
        }

        let mirror = if mirror {
            let mut entity_of = BTreeMap::new();
            entity_of.insert(n(1), hero);
            entity_of.insert(n(2), goblin);
            let mut at = BTreeMap::new();
            at.insert(hero, [Fx16_16::ZERO; 3]);
            at.insert(goblin, [Fx16_16::ZERO; 3]);
            Some(Mirror {
                world: start.mirror,
                mesh: start.mesh,
                revision: start.revision,
                entity_of,
                at,
            })
        } else {
            None
        };

        Arena {
            host_loop,
            clients,
            mirror,
            hero,
            goblin,
            cycle: 0,
        }
    }

    /// B-6: one cycle, returning its admitted lane-0 intents.
    pub fn step(&mut self) -> Vec<Admitted> {
        let c = self.cycle;
        let now = c * PERIOD_MS;
        // 1. Scripts.
        for i in 0..self.clients.len() {
            self.clients[i].run_script(c);
        }
        // 2. Up.
        let mut admitted = Vec::new();
        for i in 0..self.clients.len() {
            let peer = self.clients[i].peer;
            let frames = self.clients[i].up.take_due(c);
            for bytes in frames {
                self.clients[i].link.send(&bytes).expect("link");
                if bytes.first() == Some(&V4) {
                    let frame = codec_v4::decode_intent(&bytes).expect("decodes an intent");
                    admitted.push(Admitted {
                        client: i,
                        peer,
                        frame,
                    });
                }
            }
        }
        // 3. Loop.
        let report = self.host_loop.step(now).expect("loop");
        assert!(report.cycle.is_some(), "loop");
        assert_eq!(report.missed, 0, "loop");
        // 4. Mirror.
        if let Some(mirror) = self.mirror.as_mut() {
            for admitted_item in &admitted {
                if matches!(
                    self.clients[admitted_item.client].script,
                    Script::Intruder { .. }
                ) {
                    continue;
                }
                mirror.apply(admitted_item);
            }
            mirror.tick();
        }
        // 5. Down.
        for i in 0..self.clients.len() {
            let mut frames = Vec::new();
            while let Some(frame) = self.clients[i].link.recv() {
                frames.push(frame);
            }
            for bytes in frames {
                self.clients[i].route_down(&bytes, c);
            }
        }
        // 6. Deliver.
        for i in 0..self.clients.len() {
            self.clients[i].deliver(c);
        }
        self.cycle = c + 1;
        admitted
    }

    /// `k` steps.
    pub fn run(&mut self, cycles: u64) {
        for _ in 0..cycles {
            self.step();
        }
    }

    /// B-7: pause every script, switch every channel to `Faults::CLEAN`, run
    /// `SETTLE_CYCLES`, then require every channel empty.
    pub fn settle(&mut self) {
        for client in &mut self.clients {
            client.paused = true;
            client.saved_faults = client.faults;
            client.faults = Faults::CLEAN;
        }
        for _ in 0..SETTLE_CYCLES {
            self.step();
        }
        for client in &self.clients {
            assert_eq!(client.in_flight(), 0, "unsettled {}", client.name);
        }
    }

    /// B-7: restore the fault profile and unpause.
    pub fn resume(&mut self) {
        for client in &mut self.clients {
            client.faults = client.saved_faults;
            client.paused = false;
        }
    }

    /// The client whose `name` matches exactly; panics `"no client"`.
    pub fn client(&self, name: &str) -> &HeadlessClient {
        self.clients
            .iter()
            .find(|c| c.name == name)
            .expect("no client")
    }

    /// The client whose `name` matches exactly; panics `"no client"`.
    pub fn client_mut(&mut self, name: &str) -> &mut HeadlessClient {
        self.clients
            .iter_mut()
            .find(|c| c.name == name)
            .expect("no client")
    }

    /// The twin of `name`, `client(&format!("{name}~"))`.
    pub fn twin(&self, name: &str) -> &HeadlessClient {
        self.client(&format!("{name}~"))
    }

    /// The loop host's `authority_hash()`.
    pub fn authority(&self) -> [u8; 32] {
        self.host_loop.hosting().host().authority_hash()
    }

    /// `history_hash` of the mirror; panics `"no mirror"`.
    pub fn mirror_hash(&self) -> [u8; 32] {
        history_hash(&self.mirror.as_ref().expect("no mirror").world)
    }
}

/// Builds one headless client from its binding, link and profile.
#[allow(clippy::too_many_arguments)]
fn new_client(
    name: &str,
    peer: PeerHandle,
    epoch: [u8; 16],
    twin_of: Option<usize>,
    faults: Faults,
    script: Script,
    link: LocalClient,
    net_seed: u64,
) -> HeadlessClient {
    let mut pool = ReassemblyPoolV4::new(TransferLimitsV3::v3()).expect("v3 limits");
    let rid = pool
        .add_receiver(epoch, GateMode::AwaitSnapshot)
        .expect("receiver");
    HeadlessClient {
        name: name.to_string(),
        peer,
        epoch,
        twin_of,
        replica: ReplicaWorldV4::new(SessionEpoch(epoch)),
        outbox: MoveOutbox::new(epoch),
        faults,
        script,
        paused: false,
        sent: Vec::new(),
        receipts: BTreeMap::new(),
        receipt_cycles: Vec::new(),
        move_receipts: Vec::new(),
        gate_errors: Vec::new(),
        replica_errors: Vec::new(),
        link,
        pool,
        rid,
        up: Channel::new(ChannelKind::Reliable, net_seed, &format!("{name}/up")),
        down0: Channel::new(ChannelKind::Reliable, net_seed, &format!("{name}/down0")),
        down1: Channel::new(ChannelKind::Reliable, net_seed, &format!("{name}/down1")),
        datagram: Channel::new(ChannelKind::Datagram, net_seed, &format!("{name}/datagram")),
        next_seq: 1,
        acted: None,
        awaiting: None,
        moves_applied: 0,
        walk_leg: 0,
        saved_faults: faults,
    }
}
