//! T037e acceptance suite (`tasks/T037.md`, tests K1 to K4): authored campaign
//! data through the host built by T037a-d and a protocol v4 client.
//!
//! Content is the real authored `campaigns/fixtures/combat_basic/`, loaded through
//! the existing T016c support loader (`tests/support/combat.rs`). Its ruleset
//! is given the movement block in memory before the encounter starts (the
//! sim fixtures' pattern): speed 1 m per tick, one step per 65,536 raw, one
//! primary-pool unit per step, and, for the lane-1 case only,
//! `round_ticks: 4`. Both authored placements stand at the area corner
//! `[0, 0, 0]`, inside bounds `[0, 0, 0] ..= [65536, 65536, 65536]`.
//!
//! Oracles are independent of the host: a cloned `HistoryWorld` from the same
//! start, driven with the same sim calls (routes computed by calling
//! `crpg_nav` directly on the mesh the area bounds bake), and hand-written
//! expected pages, receipts and samples. The v4 client is the T026p5 one
//! (`ReassemblyPoolV4` feeding a `ReplicaWorldV4`) plus `MoveOutbox` for lane
//! 1; samples and receipts are decoded with `movement_v4::decode_down`.
//!
//! Contract erratum (session 74): the goblin (initiative 9) acts first in
//! first-blood, so K2 moves the goblin, not the hero.

#[path = "support/combat.rs"]
#[allow(dead_code)]
mod combat;

use std::collections::BTreeMap;

use crpg_core::{EntityId, Fx16_16, Ulid};
use crpg_data::{
    Ability, Area, Creature, Document, Effect, Encounter, LengthWire, MovementWire, OutcomeTable,
    Placement, Ruleset, TurnMovementWire,
};
use crpg_nav::{bake_box, NavMesh, NavRect};
use crpg_net::codec_v4;
use crpg_net::facts_v3::EncounterGrants;
use crpg_net::movement_v4::{
    decode_down, encode_intent, MoveBody, MoveDown, MoveIntent, MoveOutcome as MoveVerdict,
    MoveReceipt, MoveSample, SampleEntry,
};
use crpg_net::outbox_v4::MoveOutbox;
use crpg_net::pool_v4::{
    GateMode, ReassemblyPoolV4, ReceiverIdV4, ReplicaUpdateV4, TransferLimitsV3,
};
use crpg_net::protocol::{NetId, ReceiptStatus, RejectionCode, LANE_COMBAT};
use crpg_net::protocol_transfer::{TransferId, TransferUp};
use crpg_net::protocol_v3::{
    DeltaOp as DeltaOpV3, PauseRights, PoolFact, ReceiptStatus as ReceiptV3,
    RejectionCode as CodeV3,
};
use crpg_net::protocol_v4::{
    DeltaFrame as DeltaFrameV4, DeltaOp, IntentBody as IntentBodyV4, IntentFrame as IntentFrameV4,
    Point, SessionEpoch,
};
use crpg_net::replica_v4::{PositionState, ReplicaWorldV4};
use crpg_net::transfer_v3;
use crpg_server::checkpoint::load_checkpoint;
use crpg_server::host::{
    AdmissionResult, ControlGrant, DeliveryEntry, DeliveryKind, DisclosureGrants, EntityDisclosure,
    EntityFactGrant, FactGrants, Host, HostConfig, IngestDisposition, MoveAdmission, NavBinding,
    PeerHandle, ProtocolSelection, TransferDisposition, DEFAULT_HOST_GRANTS,
};
use crpg_sim::{
    history_hash, EncounterMode, EncounterSpec, HistoryWorld, MoveRoute, NavRevision,
    PlacementAndArea, RouteTag,
};

// ---------------------------------------------------------------------------
// Identities, geometry and fixtures.
// ---------------------------------------------------------------------------

/// `ulid("…")`: an authored identity out of the campaign's ULID text.
fn ulid(text: &str) -> Ulid {
    text.parse().expect("a canonical ULID")
}

/// The fixture ruleset's primary pool.
fn pool_id() -> Ulid {
    ulid("0000000000000000000000000P")
}

/// The hero fixture placement.
fn hero_placement() -> Ulid {
    ulid("0000000000000000000000000E")
}

/// The goblin fixture placement.
fn goblin_placement() -> Ulid {
    ulid("0000000000000000000000000F")
}

fn n(raw: u64) -> NetId {
    NetId::new(raw).expect("nonzero")
}

/// A raw wire point `[x, y, z]`.
fn pt(x: i32, y: i32, z: i32) -> Point {
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

/// `R(mesh)`.
fn r(mesh: &NavMesh) -> NavRevision {
    NavRevision::of_bytes(&mesh.to_canonical_bytes())
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
                pool: pool_id(),
                step: LengthWire::Constant(Fx16_16::from_raw(65536)),
                step_cost: 1,
            },
        }));
        ruleset.round_ticks = round_ticks;
        ruleset
    }
}

fn disclosure(entity: EntityId) -> EntityDisclosure {
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

fn all_fact_flags(entity: EntityId) -> EntityFactGrant {
    EntityFactGrant {
        entity,
        pools: true,
        effects: true,
        stats: true,
        tags: true,
    }
}

fn control(actors: &[EntityId]) -> ControlGrant {
    ControlGrant {
        actors: actors.to_vec(),
    }
}

fn r1() -> PauseRights {
    PauseRights {
        pause: true,
        unpause: true,
    }
}

// ---------------------------------------------------------------------------
// Rig: the host, its mirror authority and one authored area.
// ---------------------------------------------------------------------------

struct Rig {
    host: Host,
    oracle: HistoryWorld,
    ids: [EntityId; 2],
    bounds: ([Fx16_16; 3], [Fx16_16; 3]),
    mesh: NavMesh,
    ability: Ulid,
    encounter: Ulid,
    now: u64,
}

impl Rig {
    /// Loads the campaign, starts `first-blood` in `mode` under the authored
    /// movement rules, and wraps a clone in the host and the mirror.
    fn start(mode: EncounterMode, round_ticks: Option<u32>) -> Self {
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
        let mut history = HistoryWorld::new_in_area(11, content.area.id);
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
        let ids = [bound[0].0, bound[1].0];
        let oracle = history.clone();
        let bounds = (content.area.bounds.min, content.area.bounds.max);
        let mesh = area_mesh(bounds.0, bounds.1);
        let host = Host::from_history(
            history,
            HostConfig {
                protocol: ProtocolSelection::V2,
                incarnation: 1,
            },
            0,
        )
        .expect("wraps the started history");
        Self {
            host,
            oracle,
            ids,
            bounds,
            mesh,
            ability: ruleset.abilities[0],
            encounter: content.encounter.id,
            now: 0,
        }
    }

    fn revision(&self) -> NavRevision {
        r(&self.mesh)
    }

    /// The host's `bind_area_bounds` and the mirror's `bind_nav`.
    fn bind_area(&mut self) -> NavBinding {
        let binding = self
            .host
            .bind_area_bounds(self.bounds.0, self.bounds.1)
            .expect("the authored bounds bind");
        self.oracle
            .bind_nav(binding.revision)
            .expect("the mirror binds the revision");
        self.mirror_ack();
        binding
    }

    fn mirror_ack(&mut self) {
        self.oracle
            .acknowledge(self.oracle.last_sequence())
            .expect("acknowledges its own range");
    }

    fn assert_authority(&self) {
        assert_eq!(self.host.authority_hash(), history_hash(&self.oracle));
    }

    /// `P`: a v4 session over `actors`, every entity disclosed and granted.
    fn bind_p(&mut self, actors: &[EntityId]) -> PeerHandle {
        let grants = DisclosureGrants {
            entities: self.ids.iter().copied().map(disclosure).collect(),
            abilities: vec![self.ability],
            encounters: vec![self.encounter],
            turn_round: true,
            encounter_end: true,
            turn_state: true,
        };
        let facts = FactGrants {
            entities: self.ids.iter().copied().map(all_fact_flags).collect(),
            encounter: EncounterGrants {
                mode: true,
                pending_mode: true,
                clock: true,
                timeline: true,
            },
            host: DEFAULT_HOST_GRANTS,
        };
        let peer = self
            .host
            .bind_v4_peer(control(actors), grants, facts, r1(), self.now)
            .expect("binds a v4 peer");
        assert_eq!(self.host.session_wire(peer), Ok(4));
        peer
    }

    fn drain(&mut self, peer: PeerHandle) -> Vec<DeliveryEntry> {
        drain(&mut self.host, peer)
    }

    /// One v4 `DeclareMove` through `ingest` then `pump`.
    fn declare_move(
        &mut self,
        peer: PeerHandle,
        seq: u64,
        actor: NetId,
        destination: Point,
    ) -> AdmissionResult {
        let bytes = declare_move_bytes(&self.host, peer, seq, actor, destination);
        self.send(peer, &bytes)
    }

    fn send(&mut self, peer: PeerHandle, bytes: &[u8]) -> AdmissionResult {
        assert_eq!(
            self.host.ingest(peer, bytes, self.now),
            Ok(IngestDisposition::Staged)
        );
        let summary = self.host.pump(self.now).expect("pump");
        assert_eq!(summary.results.len(), 1, "{summary:?}");
        summary.results[0].clone()
    }

    /// The mirror's `perform_move` with the route nav answers, tag `tag`.
    fn mirror_move(&mut self, actor: EntityId, from: [Fx16_16; 3], to: [Fx16_16; 3], tag: u64) {
        let points = self
            .mesh
            .find_path(from, to)
            .expect("the mesh routes")
            .into_points();
        let route = MoveRoute {
            revision: self.revision(),
            tag: RouteTag(tag),
            points,
        };
        self.oracle
            .perform_move(actor, &route)
            .expect("the mirror moves");
        self.mirror_ack();
    }

    /// The mirror's `set_route` with the route nav answers, tag `tag`.
    fn mirror_route(&mut self, actor: EntityId, from: [Fx16_16; 3], to: [Fx16_16; 3], tag: u64) {
        let points = self
            .mesh
            .find_path(from, to)
            .expect("the mesh routes")
            .into_points();
        let route = MoveRoute {
            revision: self.revision(),
            tag: RouteTag(tag),
            points,
        };
        self.oracle
            .set_route(actor, route)
            .expect("the mirror routes");
        self.mirror_ack();
    }

    /// The mirror's tick, then the host/mirror hash check.
    fn mirror_tick(&mut self) {
        let facts = self.oracle.tick_with_movement().expect("the mirror ticks");
        let _ = facts.len();
        self.mirror_ack();
        self.assert_authority();
    }

    /// One `advance` that ticks, mirrored.
    fn advance(&mut self, now: u64) {
        self.now = now;
        let report = self.host.advance(now).expect("advances");
        assert!(report.ticked, "{report:?}");
        self.mirror_tick();
    }
}

/// Takes and acknowledges every pending entry of `peer`.
fn drain(host: &mut Host, peer: PeerHandle) -> Vec<DeliveryEntry> {
    let entries = host.take_delivery_entries(peer).expect("bound");
    if let Some(last) = entries.last() {
        host.acknowledge_delivery(peer, last.ordinal).expect("acks");
    }
    entries
}

// ---------------------------------------------------------------------------
// The v4 client (T026p5).
// ---------------------------------------------------------------------------

struct Client {
    epoch: [u8; 16],
    pool: ReassemblyPoolV4,
    rid: ReceiverIdV4,
    replica: ReplicaWorldV4,
    last_snapshot: Option<TransferId>,
}

impl Client {
    fn new(epoch: [u8; 16]) -> Self {
        let mut pool = ReassemblyPoolV4::new(TransferLimitsV3::v3()).expect("v3 limits");
        let rid = pool
            .add_receiver(epoch, GateMode::AwaitSnapshot)
            .expect("receiver");
        Self {
            epoch,
            pool,
            rid,
            replica: ReplicaWorldV4::new(SessionEpoch(epoch)),
            last_snapshot: None,
        }
    }

    fn drain(
        &mut self,
        host: &mut Host,
        peer: PeerHandle,
        now: u64,
    ) -> (Vec<DeliveryEntry>, Vec<DeltaFrameV4>) {
        let entries = host.take_delivery_entries(peer).expect("bound");
        for entry in &entries {
            match entry.kind {
                DeliveryKind::Transfer => {
                    self.pool
                        .offer_transfer(self.rid, &entry.bytes, now)
                        .expect("the gate accepts the frame");
                }
                DeliveryKind::Delta => {
                    self.pool
                        .offer_delta(self.rid, &entry.bytes, now)
                        .expect("the gate accepts the frame");
                }
            }
        }
        if let Some(last) = entries.last() {
            host.acknowledge_delivery(peer, last.ordinal).expect("acks");
        }
        let mut deltas = Vec::new();
        while let Some(update) = self.pool.pop_update(self.rid) {
            match update {
                ReplicaUpdateV4::Snapshot(publication) => {
                    self.last_snapshot = Some(publication.transfer);
                    self.replica.apply_snapshot(&publication).expect("applies");
                }
                ReplicaUpdateV4::Delta(frame) => {
                    self.replica.apply_delta(&frame).expect("applies");
                    deltas.push(frame);
                }
            }
        }
        (entries, deltas)
    }

    fn publish(&mut self, host: &mut Host, peer: PeerHandle, now: u64) -> TransferDisposition {
        let bytes = transfer_v3::encode_up(&TransferUp::Published {
            epoch: self.epoch,
            transfer: self.last_snapshot.expect("a snapshot was published"),
        })
        .expect("encodes");
        host.ingest_transfer(peer, &bytes, now).expect("open")
    }
}

/// Drains and publishes a fresh v4 binding.
fn bind_client(host: &mut Host, peer: PeerHandle, now: u64) -> Client {
    let mut client = Client::new(host.epoch(peer).expect("bound"));
    let (_, deltas) = client.drain(host, peer, now);
    assert_eq!(deltas.len(), 1, "the anchoring page");
    assert_eq!(
        client.publish(host, peer, now),
        TransferDisposition::Published
    );
    client
}

// ---------------------------------------------------------------------------
// Intents, samples and expected ops.
// ---------------------------------------------------------------------------

fn declare_move_bytes(
    host: &Host,
    peer: PeerHandle,
    seq: u64,
    actor: NetId,
    destination: Point,
) -> Vec<u8> {
    codec_v4::encode_intent(&IntentFrameV4 {
        epoch: host.epoch(peer).expect("bound"),
        lane: LANE_COMBAT,
        seq,
        observed_tick: host.server_tick(),
        actor,
        body: IntentBodyV4::DeclareMove { destination },
    })
    .expect("encodes")
}

/// Every sample datagram of `peer`, decoded.
fn samples(host: &mut Host, peer: PeerHandle) -> Vec<MoveSample> {
    host.take_samples(peer)
        .expect("a v4 session")
        .iter()
        .map(
            |bytes| match decode_down(bytes).expect("a move-down datagram") {
                MoveDown::Sample(sample) => sample,
                other => panic!("expected a sample, got {other:?}"),
            },
        )
        .collect()
}

/// Every released lane-1 receipt of `peer`, decoded.
fn receipts(host: &mut Host, peer: PeerHandle) -> Vec<MoveReceipt> {
    host.take_move_receipts(peer)
        .expect("a v4 session")
        .iter()
        .map(
            |bytes| match decode_down(bytes).expect("a move-down datagram") {
                MoveDown::Receipt(receipt) => receipt,
                other => panic!("expected a receipt, got {other:?}"),
            },
        )
        .collect()
}

fn receipt_op(epoch: [u8; 16], seq: u64, processed_tick: u64, status: ReceiptV3) -> DeltaOpV3 {
    DeltaOpV3::Receipt {
        epoch,
        lane: LANE_COMBAT,
        seq,
        processed_tick,
        status,
    }
}

fn moved(entity: u64, points: &[(i32, i32, i32)]) -> DeltaOp {
    DeltaOp::Moved {
        entity: n(entity),
        points: points.iter().map(|(x, y, z)| pt(*x, *y, *z)).collect(),
    }
}

fn entry_ops(entry: &DeliveryEntry) -> Vec<DeltaOp> {
    codec_v4::decode_delta(&entry.bytes)
        .expect("a v4 delta")
        .ops
}

// ---------------------------------------------------------------------------
// K1.
// ---------------------------------------------------------------------------

/// K1: the authored area's two bounds bake the nav mesh the host binds, and
/// both corner placements stay on it.
#[test]
fn k1_authored_area_binds() {
    let mut rig = Rig::start(EncounterMode::TurnBased, None);
    assert_eq!(rig.bounds.0, [Fx16_16::from_raw(0); 3]);
    assert_eq!(rig.bounds.1, [Fx16_16::from_raw(65536); 3]);
    let expected =
        r(&bake_box(Fx16_16::ZERO, rect_raw(0, 0, 65536, 65536), &[]).expect("the bounds bake"));
    assert_eq!(rig.host.nav_revision(), None);

    let binding = rig.bind_area();
    assert_eq!(
        binding,
        NavBinding {
            revision: expected,
            cleared: Vec::new(),
            placed: Vec::new(),
        }
    );
    assert_eq!(rig.host.nav_revision(), Some(expected));
    rig.assert_authority();
}

// ---------------------------------------------------------------------------
// K2.
// ---------------------------------------------------------------------------

/// K2: an authored turn-based move charges the authored step and pool through
/// the host; the v4 page carries `Moved` and the spent pool.
#[test]
fn k2_authored_turn_based_move() {
    let mut rig = Rig::start(EncounterMode::TurnBased, None);
    let (hero, goblin) = (rig.ids[0], rig.ids[1]);
    assert_ne!(hero, goblin);
    let binding = rig.bind_area();
    assert_eq!(binding.cleared, Vec::new());
    assert_eq!(binding.placed, Vec::new());
    let ids = rig.ids;
    let p = rig.bind_p(&ids);
    let mut client = bind_client(&mut rig.host, p, rig.now);
    let epoch = rig.host.epoch(p).expect("bound");

    // Contract erratum (session 74): the goblin (initiative 9) acts first in
    // first-blood, so K2 moves the goblin.
    let result = rig.declare_move(p, 1, n(2), pt(32768, 0, 32768));
    assert_eq!(result.status, ReceiptStatus::Applied);
    assert_eq!(result.seq, Some(1));
    assert!(!result.cached);
    rig.mirror_move(
        goblin,
        [Fx16_16::ZERO; 3],
        [
            Fx16_16::from_raw(32768),
            Fx16_16::ZERO,
            Fx16_16::from_raw(32768),
        ],
        1,
    );
    rig.assert_authority();

    // P's one page: the receipt, the `Moved` notice and the goblin's facts,
    // whose primary pool fell from 1 to 0.
    let (entries, deltas) = client.drain(&mut rig.host, p, rig.now);
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert_eq!(deltas.len(), 1);
    assert_eq!(deltas[0].event_seq, 2);
    assert_eq!(deltas[0].server_tick, 0);
    let ops = entry_ops(&entries[0]);
    assert_eq!(ops.len(), 3, "{ops:?}");
    assert_eq!(
        ops[0],
        DeltaOp::V3(receipt_op(epoch, 1, 0, ReceiptV3::Applied))
    );
    assert_eq!(ops[1], moved(2, &[(0, 0, 0), (32768, 0, 32768)]));
    match &ops[2] {
        DeltaOp::V3(DeltaOpV3::EntityFacts { entity, facts }) => {
            assert_eq!(*entity, n(2));
            assert_eq!(
                facts.pools,
                Some(vec![PoolFact {
                    pool: pool_id(),
                    current: 0,
                    max: 1,
                }])
            );
        }
        other => panic!("expected the goblin's facts, got {other:?}"),
    }

    // A second move by the goblin cannot afford another step.
    let before = rig.host.authority_hash();
    let second = rig.declare_move(p, 2, n(2), pt(0, 0, 32768));
    assert_eq!(
        second.status,
        ReceiptStatus::Rejected(RejectionCode::IllegalAction)
    );
    assert_eq!(second.seq, Some(2));
    assert!(!second.cached);
    assert_eq!(
        rig.host.cached_status_v3(p, 2),
        Ok(Some(ReceiptV3::Rejected(CodeV3::IllegalAction)))
    );
    assert_eq!(rig.host.authority_hash(), before);

    // One tick: the sample carries the goblin at the destination.
    rig.advance(10);
    let taken = samples(&mut rig.host, p);
    assert_eq!(
        taken,
        vec![MoveSample {
            server_tick: 1,
            processed_seq: 0,
            entries: vec![SampleEntry {
                entity: n(2),
                point: pt(32768, 0, 32768),
                moving: false,
            }],
        }]
    );
    assert_eq!(client.replica.apply_sample(&taken[0]), 1);
    assert_eq!(
        client.replica.position(n(2)),
        Some(&PositionState {
            tick: 1,
            point: pt(32768, 0, 32768),
            moving: false,
        })
    );
}

// ---------------------------------------------------------------------------
// K3.
// ---------------------------------------------------------------------------

/// K3: a real-time lane-1 walk moves at the authored speed, its receipt and
/// samples reaching the v4 client.
#[test]
fn k3_authored_real_time_walk() {
    let mut rig = Rig::start(EncounterMode::RealTime, Some(4));
    let (hero, _goblin) = (rig.ids[0], rig.ids[1]);
    let binding = rig.bind_area();
    assert_eq!(binding.cleared, Vec::new());
    let ids = rig.ids;
    let p = rig.bind_p(&[ids[0]]);
    let mut client = bind_client(&mut rig.host, p, rig.now);
    let epoch = rig.host.epoch(p).expect("bound");

    let mut outbox = MoveOutbox::new(epoch);
    assert_eq!(outbox.processed_seq(), 0);
    let intent: MoveIntent = outbox
        .send(
            n(1),
            MoveBody::MoveTo {
                destination: pt(65536, 0, 65536),
            },
        )
        .expect("the outbox mints seq 1");
    assert_eq!(intent.seq, 1);
    let bytes = encode_intent(&intent).expect("encodes");
    assert_eq!(
        rig.host.ingest_move(p, &bytes, rig.now),
        Ok(MoveAdmission::Admitted { seq: 1 })
    );
    assert_eq!(receipts(&mut rig.host, p), Vec::new());

    rig.now = 10;
    let report = rig.host.advance(rig.now).expect("advances");
    assert!(report.ticked, "{report:?}");
    let released = receipts(&mut rig.host, p);
    assert_eq!(
        released,
        vec![MoveReceipt {
            epoch,
            seq: 1,
            actor: n(1),
            processed_tick: 0,
            outcome: MoveVerdict::Accepted { start_tick: 1 },
        }]
    );
    outbox.receive(&released[0]).expect("in-order receipt");

    let taken = samples(&mut rig.host, p);
    assert_eq!(
        taken,
        vec![MoveSample {
            server_tick: 1,
            processed_seq: 1,
            entries: vec![SampleEntry {
                entity: n(1),
                point: pt(46340, 0, 46340),
                moving: true,
            }],
        }]
    );
    assert_eq!(client.replica.apply_sample(&taken[0]), 1);
    assert_eq!(
        client.replica.position(n(1)),
        Some(&PositionState {
            tick: 1,
            point: pt(46340, 0, 46340),
            moving: true,
        })
    );

    // The decision's route is the mirror's route, then tick 1.
    rig.mirror_route(
        hero,
        [Fx16_16::ZERO; 3],
        [
            Fx16_16::from_raw(65536),
            Fx16_16::ZERO,
            Fx16_16::from_raw(65536),
        ],
        1,
    );
    rig.mirror_tick();

    // Tick 2 arrives at the destination.
    rig.now = 20;
    let report = rig.host.advance(rig.now).expect("advances");
    assert!(report.ticked, "{report:?}");
    rig.mirror_tick();
    let taken = samples(&mut rig.host, p);
    assert_eq!(
        taken,
        vec![MoveSample {
            server_tick: 2,
            processed_seq: 1,
            entries: vec![SampleEntry {
                entity: n(1),
                point: pt(65536, 0, 65536),
                moving: false,
            }],
        }]
    );
    assert_eq!(client.replica.apply_sample(&taken[0]), 1);
    assert_eq!(
        client.replica.position(n(1)),
        Some(&PositionState {
            tick: 2,
            point: pt(65536, 0, 65536),
            moving: false,
        })
    );
    assert_eq!(receipts(&mut rig.host, p), Vec::new());
}

// ---------------------------------------------------------------------------
// K4.
// ---------------------------------------------------------------------------

/// K4: a checkpoint holding a version-3 `Moved` loads, and binding the same
/// authored bounds clears nothing and changes no hash.
#[test]
fn k4_authored_checkpoint() {
    let mut rig = Rig::start(EncounterMode::TurnBased, None);
    rig.bind_area();
    let ids = rig.ids;
    let p = rig.bind_p(&ids);

    // Before the move: no `Moved` envelope, so the history is version 1.
    let before = String::from_utf8(rig.host.save_checkpoint().expect("saves")).expect("utf8");
    assert!(before.contains("\"history\":{\"version\":1,"), "{before}");

    let result = rig.declare_move(p, 1, n(2), pt(32768, 0, 32768));
    assert_eq!(result.status, ReceiptStatus::Applied);
    rig.drain(p);

    // The save holds the version-3 wrapper (one committed `Moved`).
    let saved = rig.host.save_checkpoint().expect("saves");
    let text = String::from_utf8(saved.clone()).expect("utf8");
    assert!(text.contains("\"history\":{\"version\":3,"), "{text}");
    let hash = rig.host.authority_hash();

    // The mesh is not checkpointed: the restored host binds it again, and the
    // same revision clears nothing.
    let mut loaded = load_checkpoint(&saved, 2, 0).expect("the checkpoint loads");
    assert_eq!(loaded.nav_revision(), None);
    let expected = rig.revision();
    let binding = loaded
        .bind_area_bounds(rig.bounds.0, rig.bounds.1)
        .expect("the authored bounds bind");
    assert_eq!(
        binding,
        NavBinding {
            revision: expected,
            cleared: Vec::new(),
            placed: Vec::new(),
        }
    );
    assert_eq!(loaded.authority_hash(), hash);
    assert_eq!(loaded.nav_revision(), Some(expected));
}
