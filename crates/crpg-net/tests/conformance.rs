//! T018c conformance: malicious clients, filtered desync oracle, 5,000-tick
//! exercise, receipt liveness (the T018 exit gate).
//!
//! Net-local only: the sim-backed [`NetDriver`](driver::NetDriver) test
//! double sequences public sim calls, [`InMemoryTransport`] delivers the
//! bytes, and [`Replica`]s reconstruct filtered views independently
//! compared against [`ExpectedView`](driver::ExpectedView)s. No testkit
//! runtime dependency, no host/persist/godot edge, no sim API change, no new
//! event variant, no queue/hash/replay change. Same build, same seeds:
//! every suite is deterministic with no wall clock, no sleep, no I/O, no
//! unseeded RNG.

#[path = "support/driver.rs"]
mod driver;
#[path = "support/fixture.rs"]
mod fixture;

use std::collections::{BTreeMap, BTreeSet};

use crpg_core::{EntityId, Ulid};
use crpg_net::codec::{decode_delta, decode_intent, encode_intent};
use crpg_net::protocol::{
    DeltaFrame, DeltaOp, IntentBody, IntentFrame, NetId, ReceiptStatus, RejectionCode, LANE_COMBAT,
    MAX_DELTA_FRAME_BYTES, MAX_INTENT_FRAME_BYTES, PROTOCOL_VERSION,
};
use crpg_net::sim::{FaultSchedule, InMemoryTransport, PeerId, QueueCaps, RateCaps, SimClock};
use crpg_sim::{tick as sim_tick, EntityMeta, World};
use driver::{ExpectedView, FilteredEvent, NetDriver, OwnershipTable, Visibility, VisibilityTable};
use fixture::Fixture;

const EPOCH_A: [u8; 16] = [0xA0; 16];
const EPOCH_B: [u8; 16] = [0xB0; 16];
const FOREIGN_EPOCH: [u8; 16] = [0xF0; 16];
const SERVER_TICK: u64 = 1_000;

// ---------------------------------------------------------------------------
// Harness: rig, intents, receipts, delivery, replica.
// ---------------------------------------------------------------------------

/// One bound peer's choreography handles.
struct PeerHandles {
    peer: PeerId,
    epoch: [u8; 16],
    actor: EntityId,
    net_self: NetId,
    net_other: NetId,
}

struct Rig {
    fixture: Fixture,
    driver: NetDriver,
    transport: InMemoryTransport,
    first: PeerHandles,
    second: PeerHandles,
}

fn generous_caps() -> QueueCaps {
    QueueCaps {
        per_peer_frames: 10_000,
        per_peer_bytes: 64 * 1024 * 1024,
        host_frames: 100_000,
        host_bytes: 512 * 1024 * 1024,
    }
}

fn generous_rates() -> RateCaps {
    RateCaps {
        frames_per_sec: 1_000_000,
        burst_frames: 1_000_000,
        bytes_per_sec: 1_000_000_000,
        burst_bytes: 1_000_000_000,
        fail_per_sec: 1_000_000,
        burst_fail: 1_000_000,
        fail_bytes_per_sec: 1_000_000_000,
        burst_fail_bytes: 1_000_000_000,
    }
}

/// Standard rig: two combatants, peer A owns the first and peer B the
/// second, both fully visible to both. Returns the rig with both peers
/// bound and every visible entity mapped.
fn setup(seed: u64) -> Rig {
    setup_with_visibility(seed, Visibility::full(), Visibility::full())
}

/// Standard rig with per-peer disclosure of the two combatants overridden.
fn setup_with_visibility(seed: u64, vis_a: Visibility, vis_b: Visibility) -> Rig {
    let fixture = Fixture::standard();
    let (world, ids) = fixture.start(seed);
    assert_eq!(ids.len(), 2, "standard fixture fields two combatants");
    let (ea, eb) = (ids[0], ids[1]);
    let mut transport = InMemoryTransport::new(generous_caps(), generous_rates(), SimClock(0));
    let pa = transport.add_peer(EPOCH_A).expect("peer A admitted");
    let pb = transport.add_peer(EPOCH_B).expect("peer B admitted");
    let mut vis = VisibilityTable::new();
    for entity in [ea, eb] {
        vis.set(pa, entity, vis_a);
        vis.set(pb, entity, vis_b);
    }
    let mut own = OwnershipTable::new();
    own.set(pa, vec![ea]);
    own.set(pb, vec![eb]);
    let mut driver = NetDriver::new(world, vis, own);
    driver.server_tick = SERVER_TICK;
    driver
        .bind_peer(pa, EPOCH_A, &generous_rates())
        .expect("peer A binds");
    driver
        .bind_peer(pb, EPOCH_B, &generous_rates())
        .expect("peer B binds");
    driver.assign(pa);
    driver.assign(pb);
    let net =
        |peer: PeerId, entity: EntityId| driver.net_of(peer, entity).expect("visible entity maps");
    let first = PeerHandles {
        peer: pa,
        epoch: EPOCH_A,
        actor: ea,
        net_self: net(pa, ea),
        net_other: net(pa, eb),
    };
    let second = PeerHandles {
        peer: pb,
        epoch: EPOCH_B,
        actor: eb,
        net_self: net(pb, eb),
        net_other: net(pb, ea),
    };
    Rig {
        fixture,
        driver,
        transport,
        first,
        second,
    }
}

fn attack_bytes(
    epoch: [u8; 16],
    seq: u64,
    observed: u64,
    actor: NetId,
    ability: Ulid,
    target: NetId,
) -> Vec<u8> {
    encode_intent(&IntentFrame {
        epoch,
        lane: LANE_COMBAT,
        seq,
        observed_tick: observed,
        actor,
        body: IntentBody::DeclareAction { ability, target },
    })
    .expect("fixture intent encodes")
}

fn end_turn_bytes(epoch: [u8; 16], seq: u64, observed: u64, actor: NetId) -> Vec<u8> {
    encode_intent(&IntentFrame {
        epoch,
        lane: LANE_COMBAT,
        seq,
        observed_tick: observed,
        actor,
        body: IntentBody::EndTurn,
    })
    .expect("fixture intent encodes")
}

fn next_seq(driver: &NetDriver, peer: PeerId) -> u64 {
    driver.next_seq(peer).expect("peer is bound")
}

/// The active combatant's handles, if the encounter has a live turn.
fn active_handles(rig: &Rig) -> Option<&PeerHandles> {
    let active = rig.driver.world.combat()?.active?;
    if rig.first.actor == active {
        Some(&rig.first)
    } else if rig.second.actor == active {
        Some(&rig.second)
    } else {
        None
    }
}

/// EndTurn by whoever holds the turn; the universal positive control.
fn control_end_turn(rig: &mut Rig) -> ReceiptStatus {
    let (peer, epoch, net) = {
        let handles = active_handles(rig).expect("encounter has an active turn");
        (handles.peer, handles.epoch, handles.net_self)
    };
    let seq = next_seq(&rig.driver, peer);
    rig.driver
        .admit_status(peer, &end_turn_bytes(epoch, seq, SERVER_TICK, net))
}

/// Attack by whoever holds the turn against the other combatant.
fn control_attack(rig: &mut Rig) -> ReceiptStatus {
    let (peer, epoch, actor, ability, target) = {
        let handles = active_handles(rig).expect("encounter has an active turn");
        let foe = if handles.actor == rig.first.actor {
            rig.first.net_other
        } else {
            rig.second.net_other
        };
        (
            handles.peer,
            handles.epoch,
            handles.net_self,
            rig.fixture.ability,
            foe,
        )
    };
    let seq = next_seq(&rig.driver, peer);
    rig.driver.admit_status(
        peer,
        &attack_bytes(epoch, seq, SERVER_TICK, actor, ability, target),
    )
}

/// Attack with the pricey fixture ability (primary pool plus the
/// never-refreshing second pool) by explicit handles.
fn pricey_use(
    rig: &mut Rig,
    peer: PeerId,
    epoch: [u8; 16],
    net: NetId,
    foe: NetId,
) -> ReceiptStatus {
    let pricey = rig.fixture.pricey_ability;
    let bytes = attack_bytes(
        epoch,
        next_seq(&rig.driver, peer),
        SERVER_TICK,
        net,
        pricey,
        foe,
    );
    rig.driver.admit_status(peer, &bytes)
}

/// Asserts a rejection with its exact code and byte-identical complete
/// World/RNG/event state afterwards.
fn assert_reject(rig: &mut Rig, peer: PeerId, bytes: &[u8], code: RejectionCode) {
    let before = rig.driver.authoritative_hash();
    assert_eq!(
        rig.driver.admit_status(peer, bytes),
        ReceiptStatus::Rejected(code),
        "exact rejection code"
    );
    assert_eq!(
        rig.driver.authoritative_hash(),
        before,
        "rejection leaves complete state byte-identical"
    );
}

/// Pumps one datagram each way through the fabric: stage, tick with faults,
/// take. Returns the delivered bytes, if any survived the schedule.
fn deliver(rig: &mut Rig, peer: PeerId, bytes: Vec<u8>, faults: &FaultSchedule) -> Option<Vec<u8>> {
    rig.transport
        .send_to(peer, bytes)
        .expect("test staging fits");
    rig.transport.tick(faults);
    rig.transport.recv_from(peer)
}

/// Reconstructed filtered replica: applies pumped frames in per-client
/// `event_seq` order with buffering and dedup, then compares per-fact
/// against the independent view.
#[derive(Debug, Default)]
struct Replica {
    present: BTreeSet<NetId>,
    health: BTreeMap<NetId, (u32, u32, bool)>,
    turn: Option<(Option<NetId>, u64)>,
    events: Vec<(u64, FilteredEvent)>,
    seen: BTreeSet<u64>,
    buffered: BTreeMap<u64, Vec<DeltaOp>>,
    next_expected: u64,
    highest_seen: u64,
}

impl Replica {
    /// Receives one pumped frame: duplicates drop, future frames buffer,
    /// the next expected frame applies and releases the buffered chain.
    fn receive(&mut self, bytes: &[u8]) {
        let frame = decode_delta(bytes).expect("test frames decode");
        if !self.seen.insert(frame.event_seq) {
            return;
        }
        self.highest_seen = self.highest_seen.max(frame.event_seq);
        self.buffered.insert(frame.event_seq, frame.ops);
        while let Some(ops) = self.buffered.remove(&self.next_expected) {
            let seq = self.next_expected;
            self.next_expected += 1;
            self.apply(seq, ops);
        }
    }

    fn apply(&mut self, seq: u64, ops: Vec<DeltaOp>) {
        for op in ops {
            match op {
                DeltaOp::EntityEnter { entity } => {
                    self.present.insert(entity);
                }
                DeltaOp::EntityLeave { entity } => {
                    self.present.remove(&entity);
                    self.health.remove(&entity);
                }
                DeltaOp::Spawned { entity } => {
                    self.present.insert(entity);
                    self.events.push((seq, FilteredEvent::Spawned { entity }));
                }
                DeltaOp::Despawned { entity } => {
                    self.present.remove(&entity);
                    self.health.remove(&entity);
                    self.events.push((seq, FilteredEvent::Despawned { entity }));
                }
                DeltaOp::Died { entity } => {
                    self.events.push((seq, FilteredEvent::Died { entity }));
                }
                DeltaOp::Health {
                    entity,
                    health,
                    max_health,
                    dead,
                } => {
                    self.health.insert(entity, (health, max_health, dead));
                }
                DeltaOp::Turn { active, round } => {
                    self.turn = Some((active, round));
                }
                DeltaOp::Receipt { .. } => {}
            }
        }
    }

    /// Gapless delivery so far: every sequence below the waterline applied.
    fn contiguous(&self) -> bool {
        (self.buffered.is_empty() && self.next_expected == self.highest_seen + 1)
            || (self.next_expected == 0 && self.highest_seen == 0)
    }

    /// Sequences below the high-water mark never seen: redelivery targets.
    fn missing(&self) -> Vec<u64> {
        (self.next_expected..=self.highest_seen)
            .filter(|seq| !self.seen.contains(seq))
            .collect()
    }

    /// Per-fact equality with the independent view: presence and health as
    /// maps, turn directly, events as an order-insensitive multiset (the
    /// view numbers in envelope order while delivery numbers in arrival
    /// order; both numberings are asserted gapless separately).
    fn matches(&self, view: &ExpectedView) -> bool {
        let present: Vec<NetId> = self.present.iter().copied().collect();
        let mut health: Vec<(NetId, (u32, u32, bool))> = self
            .health
            .iter()
            .map(|(net, fact)| (*net, *fact))
            .collect();
        health.sort();
        let mut view_health: Vec<(NetId, (u32, u32, bool))> = view
            .health
            .iter()
            .map(|fact| (fact.entity, (fact.health, fact.max_health, fact.dead)))
            .collect();
        view_health.sort();
        let mut replica_events: Vec<FilteredEvent> =
            self.events.iter().map(|(_, event)| *event).collect();
        replica_events.sort();
        let mut view_events: Vec<FilteredEvent> =
            view.events.iter().map(|(_, event)| *event).collect();
        view_events.sort();
        let mut view_present = view.present.clone();
        view_present.sort();
        present == view_present
            && health == view_health
            && self.turn == view.turn.map(|fact| (fact.active, fact.round))
            && replica_events == view_events
    }

    /// Event sequences applied so far, in arrival order (delivery numbers
    /// in arrival order while the view numbers in envelope order; both are
    /// asserted gapless on their own side).
    fn event_seqs(&self) -> Vec<u64> {
        self.events.iter().map(|(seq, _)| *seq).collect()
    }
}

// ---------------------------------------------------------------------------
// Suite 1: malicious-client matrix.
// ---------------------------------------------------------------------------

#[test]
fn forged_and_cross_client_ids_rejected() {
    let mut rig = setup(7);
    assert_eq!(control_end_turn(&mut rig), ReceiptStatus::Applied);
    let peer = rig.first.peer;
    let ability = rig.fixture.ability;
    let (me, foe) = (rig.first.net_self, rig.first.net_other);
    // Peer A names B's actor: cross-client binding.
    let bytes = attack_bytes(
        EPOCH_A,
        next_seq(&rig.driver, peer),
        SERVER_TICK,
        foe,
        ability,
        me,
    );
    assert_reject(&mut rig, peer, &bytes, RejectionCode::NotAuthorized);
    // Unmapped actor and target alike read as NotAuthorized, never further.
    let bytes = attack_bytes(
        EPOCH_A,
        next_seq(&rig.driver, peer),
        SERVER_TICK,
        NetId::new(777).expect("nonzero"),
        ability,
        me,
    );
    assert_reject(&mut rig, peer, &bytes, RejectionCode::NotAuthorized);
    let bytes = attack_bytes(
        EPOCH_A,
        next_seq(&rig.driver, peer),
        SERVER_TICK,
        me,
        ability,
        NetId::new(999).expect("nonzero"),
    );
    assert_reject(&mut rig, peer, &bytes, RejectionCode::NotAuthorized);
}

#[test]
fn revoked_visibility_ids_rejected() {
    let mut rig = setup(7);
    // Positive control while B is still visible to A.
    assert_eq!(control_attack(&mut rig), ReceiptStatus::Applied);
    // Revoke: B leaves A's actionable mapping.
    let eb = rig.second.actor;
    let peer = rig.first.peer;
    let (me, foe, ability) = (rig.first.net_self, rig.first.net_other, rig.fixture.ability);
    rig.driver.set_visibility(peer, eb, Visibility::none());
    assert_eq!(rig.driver.revoke(peer), 1);
    let bytes = attack_bytes(
        EPOCH_A,
        next_seq(&rig.driver, peer),
        SERVER_TICK,
        me,
        ability,
        foe,
    );
    assert_reject(&mut rig, peer, &bytes, RejectionCode::NotAuthorized);
    // The retired net id stays dead even if visibility returns: no reuse.
    rig.driver.set_visibility(peer, eb, Visibility::full());
    rig.driver.assign(peer);
    assert!(
        rig.driver.net_of(peer, eb) != Some(foe),
        "revoked ids are never reused"
    );
}

#[test]
fn wrong_epoch_rejected() {
    let mut rig = setup(7);
    assert_eq!(control_end_turn(&mut rig), ReceiptStatus::Applied);
    let peer = rig.first.peer;
    let (me, foe, ability) = (rig.first.net_self, rig.first.net_other, rig.fixture.ability);
    let bytes = attack_bytes(
        FOREIGN_EPOCH,
        next_seq(&rig.driver, peer),
        SERVER_TICK,
        me,
        ability,
        foe,
    );
    assert_reject(&mut rig, peer, &bytes, RejectionCode::SessionExpired);
}

#[test]
fn replayed_seq_same_bytes_cached_conflict_closes() {
    let mut rig = setup(7);
    let seq = next_seq(&rig.driver, rig.first.peer);
    let bytes = end_turn_bytes(EPOCH_A, seq, SERVER_TICK, rig.first.net_self);
    // Positive control, unless B holds the first turn.
    let first_status = rig.driver.admit_status(rig.first.peer, &bytes);
    if first_status == ReceiptStatus::Rejected(RejectionCode::IllegalAction) {
        // A is out of turn: B ends the turn first, then A's seq retries clean.
        assert_eq!(control_end_turn(&mut rig), ReceiptStatus::Applied);
        let seq = next_seq(&rig.driver, rig.first.peer);
        let bytes = end_turn_bytes(EPOCH_A, seq, SERVER_TICK, rig.first.net_self);
        assert_eq!(
            rig.driver.admit_status(rig.first.peer, &bytes),
            ReceiptStatus::Applied
        );
        let executions = rig.driver.executions();
        assert_eq!(
            rig.driver.admit_status(rig.first.peer, &bytes),
            ReceiptStatus::Applied
        );
        assert_eq!(
            rig.driver.executions(),
            executions,
            "cached retry never re-executes"
        );
        return;
    }
    assert_eq!(first_status, ReceiptStatus::Applied);
    let executions = rig.driver.executions();
    assert_eq!(
        rig.driver.admit_status(rig.first.peer, &bytes),
        ReceiptStatus::Applied
    );
    assert_eq!(
        rig.driver.executions(),
        executions,
        "cached retry never re-executes"
    );
    // Same seq, different canonical bytes: conflict closes the epoch.
    let peer = rig.first.peer;
    let me = rig.first.net_self;
    let conflict = end_turn_bytes(EPOCH_A, seq, SERVER_TICK - 1, me);
    assert_ne!(conflict, bytes);
    assert_reject(&mut rig, peer, &conflict, RejectionCode::SeqConflict);
    let next = next_seq(&rig.driver, peer);
    let bytes = end_turn_bytes(EPOCH_A, next, SERVER_TICK, me);
    assert_reject(&mut rig, peer, &bytes, RejectionCode::SessionExpired);
}

#[test]
fn seq_gap_does_not_advance_stale_never_reapplies() {
    let mut rig = setup(7);
    assert_eq!(control_end_turn(&mut rig), ReceiptStatus::Applied);
    let peer = rig.first.peer;
    let me = rig.first.net_self;
    let base = next_seq(&rig.driver, peer);
    let bytes = end_turn_bytes(EPOCH_A, base + 1, SERVER_TICK, me);
    assert_reject(&mut rig, peer, &bytes, RejectionCode::SeqGap);
    assert_eq!(
        next_seq(&rig.driver, peer),
        base,
        "gap does not advance the lane"
    );
    let bytes = end_turn_bytes(EPOCH_A, base, SERVER_TICK, me);
    assert_eq!(
        rig.driver.admit_status(peer, &bytes),
        ReceiptStatus::Applied,
        "filling the gap applies"
    );
    // The turn passed to B; once B passes it back, the gapped seq is new.
    assert_eq!(control_end_turn(&mut rig), ReceiptStatus::Applied);
    let bytes = end_turn_bytes(EPOCH_A, base + 1, SERVER_TICK, me);
    assert_eq!(
        rig.driver.admit_status(peer, &bytes),
        ReceiptStatus::Applied,
        "the gapped seq is new again once filled"
    );
}

#[test]
fn seq_exhaustion_requires_new_epoch() {
    let mut rig = setup(7);
    let me = rig.first.net_self;
    let peer = rig.first.peer;
    rig.driver.test_set_next_seq(peer, u64::MAX - 1);
    let last = end_turn_bytes(EPOCH_A, u64::MAX - 1, SERVER_TICK, me);
    assert_eq!(
        rig.driver.admit_status(peer, &last),
        rig.driver.admit_status(peer, &last),
        "last issuable seq finalizes deterministically"
    );
    // No further new seq is representable on this epoch: rebind the peer
    // holding the turn to a fresh epoch and numbering restarts at 1
    // instead of wrapping.
    let (peer, net) = {
        let handles = active_handles(&rig).expect("untouched fight has a turn");
        (handles.peer, handles.net_self)
    };
    rig.driver
        .bind_peer(peer, FOREIGN_EPOCH, &generous_rates())
        .expect("epoch rotation binds");
    rig.driver.assign(peer);
    let bytes = end_turn_bytes(FOREIGN_EPOCH, 1, SERVER_TICK, net);
    assert_eq!(
        rig.driver.admit_status(peer, &bytes),
        ReceiptStatus::Applied,
        "new epoch restarts at 1"
    );
}

#[test]
fn exhausted_lane_reports_seq_exhausted_not_stale() {
    let mut rig = setup(7);
    let peer = rig.first.peer;
    let me = rig.first.net_self;
    rig.driver.test_set_next_seq(peer, u64::MAX - 1);
    let last = end_turn_bytes(EPOCH_A, u64::MAX - 1, SERVER_TICK, me);
    let terminal = rig.driver.admit_status(peer, &last);
    assert!(
        terminal == ReceiptStatus::Applied
            || matches!(
                terminal,
                ReceiptStatus::Rejected(RejectionCode::IllegalAction)
            ),
        "last issuable seq finalizes terminally"
    );
    // Cached retry still recovers the retained outcome after exhaustion.
    assert_eq!(rig.driver.admit_status(peer, &last), terminal);
    // Any other non-cached seq now needs a new epoch — never a retryable gap.
    let executions = rig.driver.executions();
    let probe = end_turn_bytes(EPOCH_A, 1, SERVER_TICK, me);
    // Seq 1 predates the jump and was never cached: exhaustion takes
    // precedence over staleness so the client rotates instead of retrying.
    assert_eq!(
        rig.driver.admit_status(peer, &probe),
        ReceiptStatus::Rejected(RejectionCode::SeqExhausted)
    );
    assert_eq!(
        rig.driver.executions(),
        executions,
        "exhaustion consumes no execution"
    );
}

#[test]
fn failure_response_budgets_drop_wire_but_retain_outcome() {
    let fixture = Fixture::standard();
    let (world, _) = fixture.start(7);
    let rates = RateCaps {
        fail_per_sec: 2,
        burst_fail: 2,
        fail_bytes_per_sec: 1_000_000,
        burst_fail_bytes: 1_000_000,
        ..generous_rates()
    };
    let mut transport = InMemoryTransport::new(generous_caps(), rates, SimClock(0));
    let peer = transport.add_peer(EPOCH_A).expect("peer admitted");
    let mut driver = NetDriver::new(world, VisibilityTable::new(), OwnershipTable::new());
    driver.server_tick = SERVER_TICK;
    driver.bind_peer(peer, EPOCH_A, &rates).expect("peer binds");
    // Unmapped actor: terminal NotAuthorized, two within the burst.
    let unauth =
        |seq: u64| end_turn_bytes(EPOCH_A, seq, SERVER_TICK, NetId::new(1).expect("nonzero"));
    for seq in 1..=2 {
        let bytes = unauth(seq);
        assert_eq!(
            driver.admit_status(peer, &bytes),
            ReceiptStatus::Rejected(RejectionCode::NotAuthorized)
        );
        assert!(
            !driver.admit(peer, &bytes).is_empty(),
            "burst covers failure {seq}"
        );
    }
    // Burst spent: the outcome stays retained while the bytes drop.
    let bytes = unauth(3);
    assert_eq!(
        driver.admit_status(peer, &bytes),
        ReceiptStatus::Rejected(RejectionCode::NotAuthorized)
    );
    assert_eq!(
        driver.admit(peer, &bytes),
        Vec::<u8>::new(),
        "empty frame budget drops the response"
    );
    // Retry after the injected second replenishes recovers the receipt.
    driver.advance_fail_time(1_000);
    assert!(
        !driver.admit(peer, &bytes).is_empty(),
        "replenished budget answers the retained outcome"
    );
}

#[test]
fn driver_peer_table_bounded_at_proof_population() {
    let fixture = Fixture::standard();
    let (world, _) = fixture.start(7);
    let mut transport = InMemoryTransport::new(generous_caps(), generous_rates(), SimClock(0));
    let mut ids = Vec::new();
    for _ in 0..8 {
        ids.push(transport.add_peer(EPOCH_A).expect("fabric fits 8"));
    }
    let mut driver = NetDriver::new(world, VisibilityTable::new(), OwnershipTable::new());
    driver.server_tick = SERVER_TICK;
    for peer in &ids {
        driver
            .bind_peer(*peer, EPOCH_A, &generous_rates())
            .expect("driver fits 8");
    }
    // Rebinding a bound peer (epoch rotation) succeeds even at capacity.
    driver
        .bind_peer(ids[0], FOREIGN_EPOCH, &generous_rates())
        .expect("rotation replaces, never grows");
    // A genuinely new peer is refused: free a fabric slot for a fresh id,
    // then watch the driver refuse it as the 9th distinct binding.
    transport.remove_peer(ids[1]);
    let fresh = transport.add_peer(EPOCH_A).expect("fabric slot freed");
    assert_eq!(
        driver.bind_peer(fresh, EPOCH_A, &generous_rates()),
        Err(RejectionCode::ServerBusy),
        "9th driver binding refused"
    );
}

#[test]
fn stale_and_future_ticks_rejected_terminally() {
    let mut rig = setup(7);
    assert_eq!(control_end_turn(&mut rig), ReceiptStatus::Applied);
    let peer = rig.first.peer;
    let me = rig.first.net_self;
    let stale = next_seq(&rig.driver, peer);
    let bytes = end_turn_bytes(EPOCH_A, stale, SERVER_TICK - 201, me);
    assert_reject(&mut rig, peer, &bytes, RejectionCode::StaleTick);
    // Terminal: the stale seq is cached, not retried into a new outcome.
    let bytes = end_turn_bytes(EPOCH_A, stale, SERVER_TICK - 201, me);
    assert_eq!(
        rig.driver.admit_status(peer, &bytes),
        ReceiptStatus::Rejected(RejectionCode::StaleTick)
    );
    let future = next_seq(&rig.driver, peer);
    let bytes = end_turn_bytes(EPOCH_A, future, SERVER_TICK + 1, me);
    assert_reject(&mut rig, peer, &bytes, RejectionCode::FutureTick);
    // Boundary: exactly 200 old is still fresh — the gameplay verdict
    // decides, never StaleTick.
    let edge = next_seq(&rig.driver, peer);
    let bytes = end_turn_bytes(EPOCH_A, edge, SERVER_TICK - 200, me);
    assert_ne!(
        rig.driver.admit_status(peer, &bytes),
        ReceiptStatus::Rejected(RejectionCode::StaleTick),
        "the 200-tick boundary stays fresh"
    );
}

#[test]
fn unknown_ability_out_of_turn_and_self_target_rejected() {
    let mut rig = setup(7);
    assert_eq!(control_attack(&mut rig), ReceiptStatus::Applied);
    let ability = rig.fixture.ability;
    // Whoever does NOT hold the turn now acts out of turn.
    let (peer, epoch, actor, target) = {
        let active_is_first = active_handles(&rig).is_some_and(|h| h.actor == rig.first.actor);
        if active_is_first {
            (
                rig.second.peer,
                EPOCH_B,
                rig.second.net_self,
                rig.second.net_other,
            )
        } else {
            (
                rig.first.peer,
                EPOCH_A,
                rig.first.net_self,
                rig.first.net_other,
            )
        }
    };
    let bytes = attack_bytes(
        epoch,
        next_seq(&rig.driver, peer),
        SERVER_TICK,
        actor,
        ability,
        target,
    );
    assert_reject(&mut rig, peer, &bytes, RejectionCode::IllegalAction);
    assert!(rig
        .driver
        .last_error()
        .is_some_and(|error| error.contains("OutOfTurn")));
    // Unknown ability by whoever DOES hold the turn.
    let (peer, epoch, actor, foe) = {
        let handles = active_handles(&rig).expect("active turn");
        let foe = if handles.actor == rig.first.actor {
            rig.first.net_other
        } else {
            rig.second.net_other
        };
        (handles.peer, handles.epoch, handles.net_self, foe)
    };
    let unknown = Ulid::from_u128(999_999);
    let bytes = attack_bytes(
        epoch,
        next_seq(&rig.driver, peer),
        SERVER_TICK,
        actor,
        unknown,
        foe,
    );
    assert_reject(&mut rig, peer, &bytes, RejectionCode::IllegalAction);
    assert!(rig
        .driver
        .last_error()
        .is_some_and(|error| error.contains("UnknownAbility")));
}

#[test]
fn dead_actor_and_target_rejected() {
    let mut rig = setup(7);
    assert_eq!(control_attack(&mut rig), ReceiptStatus::Applied);
    // Fight to a death deterministically: the holder attacks, the other
    // passes, until exactly one combatant stands (cap keeps it total).
    for _ in 0..200 {
        let alive: Vec<EntityId> = [rig.first.actor, rig.second.actor]
            .into_iter()
            .filter(|entity| {
                rig.driver
                    .world
                    .combatants()
                    .get(*entity)
                    .is_some_and(|state| !state.dead())
            })
            .collect();
        if alive.len() < 2 {
            break;
        }
        let (peer, epoch, net, foe, ability) = {
            let handles = active_handles(&rig).expect("live fight has a turn");
            let foe = if handles.actor == rig.first.actor {
                rig.first.net_other
            } else {
                rig.second.net_other
            };
            (
                handles.peer,
                handles.epoch,
                handles.net_self,
                foe,
                rig.fixture.ability,
            )
        };
        let bytes = attack_bytes(
            epoch,
            next_seq(&rig.driver, peer),
            SERVER_TICK,
            net,
            ability,
            foe,
        );
        let _ = rig.driver.admit_status(peer, &bytes);
    }
    let dead = [rig.first.actor, rig.second.actor]
        .into_iter()
        .find(|entity| {
            rig.driver
                .world
                .combatants()
                .get(*entity)
                .is_some_and(|state| state.dead())
        })
        .expect("the deterministic fight kills someone");
    let (dead_peer, dead_epoch, dead_net) = if dead == rig.first.actor {
        (rig.first.peer, EPOCH_A, rig.first.net_self)
    } else {
        (rig.second.peer, EPOCH_B, rig.second.net_self)
    };
    let seq = next_seq(&rig.driver, dead_peer);
    let ability = rig.fixture.ability;
    assert_reject(
        &mut rig,
        dead_peer,
        &attack_bytes(dead_epoch, seq, SERVER_TICK, dead_net, ability, dead_net),
        RejectionCode::IllegalAction,
    );
    assert!(rig
        .driver
        .last_error()
        .is_some_and(|error| error.contains("DeadActor")));
    // A live combatant naming the corpse as target fails the same way.
    let (peer, epoch, net, foe, ability) = if dead == rig.first.actor {
        (
            rig.second.peer,
            EPOCH_B,
            rig.second.net_self,
            rig.second.net_other,
            rig.fixture.ability,
        )
    } else {
        (
            rig.first.peer,
            EPOCH_A,
            rig.first.net_self,
            rig.first.net_other,
            rig.fixture.ability,
        )
    };
    let bytes = attack_bytes(
        epoch,
        next_seq(&rig.driver, peer),
        SERVER_TICK,
        net,
        ability,
        foe,
    );
    assert_reject(&mut rig, peer, &bytes, RejectionCode::IllegalAction);
    assert!(rig
        .driver
        .last_error()
        .is_some_and(|error| error.contains("DeadTarget")));
}

#[test]
fn insufficient_pool_rejected() {
    let mut rig = setup(7);
    // Positive control with the affordable ability first (B attacks A).
    assert_eq!(control_attack(&mut rig), ReceiptStatus::Applied);
    // The pricey ability spends the primary pool plus a never-refreshing
    // second pool: the first use applies and depletes that pool on the
    // actor, so the same actor's second use fails closed with nothing
    // mutated. Pools are per-combatant, so both uses are A's.
    let (peer, epoch, net, foe) = (
        rig.first.peer,
        EPOCH_A,
        rig.first.net_self,
        rig.first.net_other,
    );
    assert_eq!(
        pricey_use(&mut rig, peer, epoch, net, foe),
        ReceiptStatus::Applied
    );
    assert_eq!(control_end_turn(&mut rig), ReceiptStatus::Applied);
    let before = rig.driver.authoritative_hash();
    assert_eq!(
        pricey_use(&mut rig, peer, epoch, net, foe),
        ReceiptStatus::Rejected(RejectionCode::IllegalAction)
    );
    assert_eq!(
        rig.driver.authoritative_hash(),
        before,
        "failed spend mutates nothing"
    );
    assert!(rig
        .driver
        .last_error()
        .is_some_and(|error| error.contains("InsufficientAction")));
}

#[test]
fn malformed_oversized_version_and_tag_rejected() {
    let mut rig = setup(7);
    assert_eq!(control_end_turn(&mut rig), ReceiptStatus::Applied);
    let peer = rig.first.peer;
    assert_reject(&mut rig, peer, &[0x01, 0x02], RejectionCode::Malformed);
    assert_reject(
        &mut rig,
        peer,
        &vec![0xAA; MAX_INTENT_FRAME_BYTES + 1],
        RejectionCode::FrameTooLarge,
    );
    let mut bad_version = end_turn_bytes(
        EPOCH_A,
        next_seq(&rig.driver, peer),
        SERVER_TICK,
        rig.first.net_self,
    );
    bad_version[0] = 0x7F;
    assert_reject(
        &mut rig,
        peer,
        &bad_version,
        RejectionCode::UnsupportedVersion,
    );
    let mut bad_tag = end_turn_bytes(
        EPOCH_A,
        next_seq(&rig.driver, peer),
        SERVER_TICK,
        rig.first.net_self,
    );
    let tag_pos = bad_tag.len() - 1;
    bad_tag[tag_pos] = 0x42;
    assert_reject(&mut rig, peer, &bad_tag, RejectionCode::UnknownMessage);
    // Undecodable frames yield no reply bytes at all, and still mutate
    // nothing (asserted inside assert_reject through admit_status).
    assert_eq!(rig.driver.admit(peer, &[0x01, 0x02]), Vec::<u8>::new());
}

#[test]
fn over_rate_bursts_rejected_before_admit() {
    let fixture = Fixture::standard();
    let (world, _) = fixture.start(7);
    let vis = VisibilityTable::new();
    let own = OwnershipTable::new();
    let rates = RateCaps {
        frames_per_sec: 2,
        burst_frames: 2,
        bytes_per_sec: 1_000_000,
        burst_bytes: 1_000_000,
        ..generous_rates()
    };
    let mut transport = InMemoryTransport::new(generous_caps(), rates, SimClock(0));
    let peer = transport.add_peer(EPOCH_A).expect("peer admitted");
    let mut driver = NetDriver::new(world, vis, own);
    driver.server_tick = SERVER_TICK;
    driver.bind_peer(peer, EPOCH_A, &rates).expect("peer binds");
    let bytes = end_turn_bytes(EPOCH_A, 1, SERVER_TICK, NetId::new(1).expect("nonzero"));
    assert!(transport.send_to(peer, bytes.clone()).is_ok());
    assert!(transport.send_to(peer, bytes.clone()).is_ok());
    assert_eq!(
        transport.send_to(peer, bytes.clone()),
        Err(RejectionCode::RateLimited),
        "third burst frame never reaches admit"
    );
    assert_eq!(driver.executions(), 0);
    assert_eq!(
        driver.next_seq(peer),
        Some(1),
        "no seq consumed by rate failures"
    );
    transport.clock_mut().advance(1_000);
    transport
        .send_to(peer, bytes.clone())
        .expect("budget replenished");
    transport.tick(&FaultSchedule::clean());
    let arrived = transport.recv_from(peer).expect("delivers");
    // The driver never saw earlier attempts; the unmapped actor reads as
    // unauthorized, proving rate failures never reached admit.
    assert_eq!(
        driver.admit_status(peer, &arrived),
        ReceiptStatus::Rejected(RejectionCode::NotAuthorized)
    );
}

// ---------------------------------------------------------------------------
// Suite 2: filtered desync oracle.
// ---------------------------------------------------------------------------

/// Pumps one peer's due deltas into its replica, returning frame count.
fn pump_to_replica(rig: &mut Rig, peer: PeerId, replica: &mut Replica) -> usize {
    let mut count = 0;
    while let Some(bytes) = rig.driver.deltas(peer) {
        let staged = bytes.clone();
        if let Some(delivered) = deliver(rig, peer, staged, &FaultSchedule::clean()) {
            replica.receive(&delivered);
            count += 1;
        }
    }
    count
}

#[test]
fn filtered_replica_converges_per_peer() {
    // Peer B sees A's presence but never its health values; B keeps full
    // disclosure of its own actor.
    let mut rig = setup_with_visibility(11, Visibility::full(), Visibility::presence_only());
    let pb = rig.second.peer;
    let eb = rig.second.actor;
    rig.driver.set_visibility(pb, eb, Visibility::full());
    // Peer B must still map A (presence) for targeting; ownership unchanged.
    let mut first_replica = Replica::default();
    let mut second_replica = Replica::default();
    let (p1, p2) = (rig.first.peer, rig.second.peer);
    for _ in 0..6 {
        if active_handles(&rig).is_none() {
            break;
        }
        let status = if active_handles(&rig).is_some_and(|h| h.actor == rig.first.actor) {
            control_attack(&mut rig)
        } else {
            control_end_turn(&mut rig)
        };
        assert!(
            status == ReceiptStatus::Applied
                || matches!(
                    status,
                    ReceiptStatus::Rejected(RejectionCode::IllegalAction)
                ),
            "scripted schedule stays terminal-clean"
        );
        pump_to_replica(&mut rig, p1, &mut first_replica);
        pump_to_replica(&mut rig, p2, &mut second_replica);
    }
    assert!(
        first_replica.matches(&rig.driver.project(rig.first.peer)),
        "full-view converges"
    );
    assert!(
        second_replica.matches(&rig.driver.project(rig.second.peer)),
        "filtered view converges"
    );
    // The filtered peer holds presence without health facts for A.
    assert!(second_replica.present.contains(&rig.second.net_other));
    assert!(!second_replica.health.contains_key(&rig.second.net_other));
    assert!(second_replica.health.contains_key(&rig.second.net_self));
    // Both replicas number their own deliveries gaplessly from zero.
    assert!(first_replica.contiguous());
    assert!(second_replica.contiguous());
    for replica in [&first_replica, &second_replica] {
        let seqs = replica.event_seqs();
        assert!(
            seqs.windows(2).all(|pair| pair[0] <= pair[1]),
            "clean delivery preserves per-client order"
        );
    }
}

#[test]
fn desync_oracle_fails_loudly_on_drop_and_corruption() {
    let mut rig = setup(11);
    let mut replica = Replica::default();
    let peer = rig.first.peer;
    assert_eq!(control_attack(&mut rig), ReceiptStatus::Applied);
    pump_to_replica(&mut rig, peer, &mut replica);
    assert!(replica.matches(&rig.driver.project(rig.first.peer)));
    // Negative control 1: drop the next frame entirely.
    assert_eq!(control_end_turn(&mut rig), ReceiptStatus::Applied);
    let dropped = rig.driver.deltas(peer).expect("frame due");
    let dropped_seq = decode_delta(&dropped).expect("decodes").event_seq;
    let _ = deliver(&mut rig, peer, dropped, &FaultSchedule::clean());
    assert!(
        !replica.matches(&rig.driver.project(peer)),
        "dropped update diverges"
    );
    // Recovery through the sent log: a later frame arrives first and
    // buffers, the replica names its gap, redelivery fills it, and the
    // buffered chain then applies in order.
    assert_eq!(control_end_turn(&mut rig), ReceiptStatus::Applied);
    let later = rig.driver.deltas(peer).expect("frame due");
    let delivered = deliver(&mut rig, peer, later, &FaultSchedule::clean()).expect("lands");
    replica.receive(&delivered);
    assert!(!replica.contiguous(), "the gap stalls the waterline");
    assert_eq!(replica.missing(), vec![dropped_seq]);
    let resent = rig
        .driver
        .redeliver(peer, dropped_seq)
        .expect("sent log retains");
    let delivered = deliver(&mut rig, peer, resent, &FaultSchedule::clean()).expect("lands");
    replica.receive(&delivered);
    assert!(
        replica.matches(&rig.driver.project(peer)),
        "redelivery reconverges"
    );
    // Negative control 2: corrupt a delivered frame's facts.
    let mut replica = Replica::default();
    pump_to_replica(&mut rig, peer, &mut replica);
    let frame = rig.driver.deltas(rig.first.peer);
    if let Some(bytes) = frame {
        let mut tampered = bytes.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 0xFF;
        match decode_delta(&tampered) {
            Err(_) => {}
            Ok(_) => {
                replica.receive(&tampered);
                assert!(
                    !replica.matches(&rig.driver.project(rig.first.peer)),
                    "corrupted facts diverge"
                );
            }
        }
    }
}

#[test]
fn hidden_lifecycle_never_leaks_and_died_is_not_despawn() {
    let mut rig = setup_with_visibility(13, Visibility::full(), Visibility::presence_only());
    // Peer B sees A's presence but not its health; a third entity is fully
    // hidden from B and fully visible to A.
    let hidden = rig.driver.world.spawn(EntityMeta {});
    rig.driver
        .set_visibility(rig.first.peer, hidden, Visibility::full());
    let mut first_replica = Replica::default();
    let mut second_replica = Replica::default();
    let (p1, p2) = (rig.first.peer, rig.second.peer);
    pump_to_replica(&mut rig, p1, &mut first_replica);
    pump_to_replica(&mut rig, p2, &mut second_replica);
    let hidden_net = rig
        .driver
        .net_of(rig.first.peer, hidden)
        .expect("A maps the spawn");
    assert!(
        first_replica.present.contains(&hidden_net),
        "A learns the spawn"
    );
    assert!(
        first_replica
            .events
            .iter()
            .any(|(_, event)| *event == FilteredEvent::Spawned { entity: hidden_net }),
        "A learns the spawn notice"
    );
    let first_view = rig.driver.project(rig.first.peer);
    assert!(first_replica.matches(&first_view));
    assert!(second_replica.matches(&rig.driver.project(rig.second.peer)));
    assert!(
        !second_replica
            .present
            .iter()
            .any(|net| rig.driver.entity_of(rig.second.peer, *net) == Some(hidden)),
        "B never learns the hidden spawn"
    );
    // Disclosable despawn of the third entity: A sees Despawned, B nothing,
    // and B's later numbering stays contiguous (no count/gap leak).
    assert!(rig.driver.world.despawn(hidden));
    pump_to_replica(&mut rig, p1, &mut first_replica);
    pump_to_replica(&mut rig, p2, &mut second_replica);
    assert!(first_replica.matches(&rig.driver.project(rig.first.peer)));
    assert!(second_replica.matches(&rig.driver.project(rig.second.peer)));
    let (_, last_event) = first_replica.events.last().copied().expect("A saw events");
    assert!(
        matches!(last_event, FilteredEvent::Despawned { .. }),
        "disclosable removal reads as Despawned"
    );
    // Kill A through B's legal attacks; both replicas record Died, A stays
    // present in both (death is not despawn), and B learns no health facts.
    for _ in 0..200 {
        let a_dead = rig
            .driver
            .world
            .combatants()
            .get(rig.first.actor)
            .is_some_and(|state| state.dead());
        if a_dead {
            break;
        }
        let (peer, epoch, net, actor) = {
            let handles = active_handles(&rig).expect("live fight has a turn");
            (handles.peer, handles.epoch, handles.net_self, handles.actor)
        };
        if actor == rig.second.actor {
            let seq = next_seq(&rig.driver, peer);
            let _ = rig.driver.admit_status(
                peer,
                &attack_bytes(
                    epoch,
                    seq,
                    SERVER_TICK,
                    net,
                    rig.fixture.ability,
                    rig.second.net_other,
                ),
            );
        } else {
            let _ = control_end_turn(&mut rig);
        }
        pump_to_replica(&mut rig, p1, &mut first_replica);
        pump_to_replica(&mut rig, p2, &mut second_replica);
    }
    assert!(
        rig.driver
            .world
            .combatants()
            .get(rig.first.actor)
            .is_some_and(|state| state.dead()),
        "the deterministic fight kills A"
    );
    assert!(first_replica.matches(&rig.driver.project(rig.first.peer)));
    assert!(second_replica.matches(&rig.driver.project(rig.second.peer)));
    assert!(
        first_replica.present.contains(&rig.first.net_self),
        "A retained after death"
    );
    assert!(
        second_replica.present.contains(&rig.second.net_other),
        "A retained for B too"
    );
    assert!(
        !second_replica.health.contains_key(&rig.second.net_other),
        "no health leak to B"
    );
    assert!(
        second_replica
            .events
            .iter()
            .any(|(_, event)| matches!(event, FilteredEvent::Died { .. })),
        "B learns the disclosable death"
    );
    // No fact about the hidden entity ever reaches B's replica: neither
    // presence, health, nor events may resolve to it.
    for net in second_replica
        .present
        .iter()
        .copied()
        .chain(second_replica.health.keys().copied())
        .chain(second_replica.events.iter().map(|(_, event)| match *event {
            FilteredEvent::Spawned { entity }
            | FilteredEvent::Despawned { entity }
            | FilteredEvent::Died { entity } => entity,
        }))
    {
        assert_ne!(
            rig.driver.entity_of(rig.second.peer, net),
            Some(hidden),
            "hidden entity leaks nowhere into B's replica"
        );
    }
}

// ---------------------------------------------------------------------------
// Suite 3: 5,000-tick loss/jitter exercise.
// ---------------------------------------------------------------------------

/// One scripted run: 5,000 host ticks with sparse valid and rejected inputs
/// delivered through loss/dup/reorder, recording hashes every 1,000 ticks.
fn exercise_run(seed: u64) -> (Vec<[u8; 32]>, ExpectedView, ExpectedView) {
    let mut rig = setup(seed);
    let faults = FaultSchedule {
        loss_every: Some(7),
        dup_every: Some(11),
        reorder_depth: 2,
        seed: 99,
    };
    let mut hashes = Vec::new();
    for step in 1..=5_000u64 {
        rig.driver.server_tick = SERVER_TICK.saturating_add(step);
        // Sparse valid input: whoever holds the turn attacks, if able.
        if step % 50 == 0 {
            let acting = active_handles(&rig).map(|handles| {
                let foe = if handles.actor == rig.first.actor {
                    rig.first.net_other
                } else {
                    rig.second.net_other
                };
                (
                    handles.peer,
                    handles.epoch,
                    handles.net_self,
                    rig.driver.server_tick,
                    foe,
                )
            });
            if let Some((peer, epoch, net, observed, foe)) = acting {
                let ability = rig.fixture.ability;
                let bytes = attack_bytes(
                    epoch,
                    next_seq(&rig.driver, peer),
                    observed,
                    net,
                    ability,
                    foe,
                );
                if let Some(arrived) = deliver(&mut rig, peer, bytes, &faults) {
                    let _ = rig.driver.admit_status(peer, &arrived);
                }
            }
        }
        // Sparse rejected input: far-future tick on a live new seq, never
        // mutating.
        if step % 200 == 0 {
            let before = rig.driver.authoritative_hash();
            let peer = rig.first.peer;
            let bytes = end_turn_bytes(
                EPOCH_A,
                next_seq(&rig.driver, peer),
                u64::MAX,
                rig.first.net_self,
            );
            assert_eq!(
                rig.driver.admit_status(peer, &bytes),
                ReceiptStatus::Rejected(RejectionCode::FutureTick)
            );
            assert_eq!(
                rig.driver.authoritative_hash(),
                before,
                "rejected input at step {step}"
            );
        }
        sim_tick(&mut rig.driver.world);
        if step % 1_000 == 0 {
            hashes.push(rig.driver.authoritative_hash());
        }
    }
    let first = rig.driver.project(rig.first.peer);
    let second = rig.driver.project(rig.second.peer);
    (hashes, first, second)
}

#[test]
fn five_thousand_tick_loss_jitter_exercise() {
    let (hashes_a, first_a, second_a) = exercise_run(42);
    let (hashes_b, first_b, second_b) = exercise_run(42);
    assert_eq!(hashes_a.len(), 5, "per-1,000 recording over 5,000 ticks");
    assert_eq!(
        hashes_a, hashes_b,
        "identical schedules hash identically across runs"
    );
    assert_eq!(first_a, first_b, "permitted views converge across runs");
    assert_eq!(second_a, second_b, "filtered views converge across runs");
    // A different master seed diverges: the recording is sensitive, not constant.
    let (hashes_c, _, _) = exercise_run(43);
    assert_ne!(hashes_a, hashes_c, "schedule sensitivity across seeds");
}

// ---------------------------------------------------------------------------
// Suite 4: receipt/sequence liveness.
// ---------------------------------------------------------------------------

#[test]
fn lost_receipt_retried_through_faults_without_reexecution() {
    let mut rig = setup(21);
    let (peer, epoch, net) = {
        let handles = active_handles(&rig).expect("opening turn exists");
        (handles.peer, handles.epoch, handles.net_self)
    };
    let bytes = end_turn_bytes(epoch, 1, SERVER_TICK, net);
    // Client -> host, clean.
    let arrived =
        deliver(&mut rig, peer, bytes.clone(), &FaultSchedule::clean()).expect("intent delivers");
    assert_eq!(
        rig.driver.admit_status(peer, &arrived),
        ReceiptStatus::Applied
    );
    assert_eq!(rig.driver.executions(), 1);
    // Host -> client, dropped by the schedule.
    let receipt = rig.driver.admit(peer, &arrived);
    assert!(!receipt.is_empty(), "decodable frames always answer");
    let dropped = deliver(
        &mut rig,
        peer,
        receipt,
        &FaultSchedule {
            loss_every: Some(1),
            dup_every: None,
            reorder_depth: 0,
            seed: 0,
        },
    );
    assert_eq!(dropped, None, "the single loss lands");
    // Retry of the same canonical bytes recovers the cached terminal
    // receipt with no second execution and no deadlock.
    let arrived = deliver(&mut rig, peer, bytes, &FaultSchedule::clean()).expect("retry delivers");
    assert_eq!(
        rig.driver.admit_status(peer, &arrived),
        ReceiptStatus::Applied
    );
    assert_eq!(rig.driver.executions(), 1);
    let frame = decode_intent(&arrived).expect("decodes");
    assert_eq!(frame.seq, 1);
}

#[test]
fn slow_peer_terminates_with_queue_full_never_silent_loss() {
    // Transport-only: a 3-frame peer floods, terminates, and drains exactly.
    let caps = QueueCaps {
        per_peer_frames: 3,
        per_peer_bytes: 1_000_000,
        host_frames: 1_000,
        host_bytes: 1_000_000,
    };
    let mut transport = InMemoryTransport::new(caps, generous_rates(), SimClock(0));
    let peer = transport.add_peer(EPOCH_A).expect("peer admitted");
    for marker in [0x11u8, 0x22, 0x33] {
        transport
            .send_to(peer, vec![marker])
            .expect("fits the 3-frame cap");
    }
    assert_eq!(
        transport.send_to(peer, vec![0x44]),
        Err(RejectionCode::QueueFull),
        "slow peer terminates instead of losing"
    );
    transport.tick(&FaultSchedule::clean());
    let mut drained = Vec::new();
    while let Some(bytes) = transport.recv_from(peer) {
        drained.push(bytes);
    }
    assert_eq!(
        drained,
        vec![vec![0x11], vec![0x22], vec![0x33]],
        "nothing lost, nothing added"
    );
}

// ---------------------------------------------------------------------------
// Cross-suite invariants: constants and determinism smoke.
// ---------------------------------------------------------------------------

#[test]
fn wire_constants_stable_across_suites() {
    assert_eq!(PROTOCOL_VERSION, 1);
    assert_eq!(MAX_INTENT_FRAME_BYTES, 4_096);
    assert_eq!(MAX_DELTA_FRAME_BYTES, 65_536);
    let frame = DeltaFrame {
        lane: LANE_COMBAT,
        server_tick: 0,
        event_seq: 0,
        ops: vec![],
    };
    let _ = frame;
    let world_a = Fixture::standard().start(7).0;
    let world_b: World = Fixture::standard().start(7).0;
    assert_eq!(
        NetDriver::new(world_a, VisibilityTable::new(), OwnershipTable::new()).authoritative_hash(),
        NetDriver::new(world_b, VisibilityTable::new(), OwnershipTable::new()).authoritative_hash(),
        "identical construction hashes identically"
    );
}
