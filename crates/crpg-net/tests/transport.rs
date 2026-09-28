//! T018b simulated-transport conformance: backpressure, rate, retry, faults.
//!
//! Net-local only: a canned test-double host owns per-peer [`PeerState`]
//! (epoch match, per-lane seq/cache, 200-tick freshness, canned legality —
//! no sim edge, no World mutation) while [`InMemoryTransport`] stages and
//! pumps the bytes. Fixed tables only: no `HashMap`, no clock, no threads,
//! no I/O, no unseeded RNG.

use std::collections::{BTreeMap, BTreeSet};

use crpg_core::Ulid;
use crpg_net::codec::{decode_delta, decode_intent, encode_delta, encode_intent, CodecError};
use crpg_net::protocol::{
    DeltaFrame, DeltaOp, IntentBody, IntentFrame, NetId, ReceiptStatus, RejectionCode, LANE_COMBAT,
    MAX_DELTA_FRAME_BYTES, MAX_SNAPSHOT_CHUNKS, POLICY_EGRESS_BYTES_HOST,
    POLICY_EGRESS_BYTES_PER_PEER, POLICY_EGRESS_FRAMES_PER_PEER, POLICY_FAIL_BYTES_PER_SEC,
    POLICY_FAIL_BYTE_BURST, POLICY_FAIL_FRAMES_PER_SEC, POLICY_FAIL_FRAME_BURST,
    POLICY_INGRESS_BYTES_HOST, POLICY_INGRESS_BYTES_PER_PEER, POLICY_INGRESS_FRAMES_HOST,
    POLICY_INGRESS_FRAMES_PER_PEER, POLICY_INTENT_BYTES_PER_SEC, POLICY_INTENT_BYTE_BURST,
    POLICY_INTENT_FRAMES_PER_SEC, POLICY_INTENT_FRAME_BURST, POLICY_OBSERVED_TICK_WINDOW,
    POLICY_PROOF_PEERS, POLICY_RETRY_CACHE_BYTES_PER_PEER_LANE, POLICY_RETRY_CACHE_PER_PEER_LANE,
    POLICY_SNAPSHOT_IN_FLIGHT_PER_PEER, POLICY_SNAPSHOT_REASSEMBLY_SECS, SEQ_FIRST, SEQ_LAST,
};
use crpg_net::sim::{
    FaultSchedule, InMemoryTransport, PeerId, PeerState, QueueCaps, RateCaps, SimClock, TokenBucket,
};

const EPOCH: [u8; 16] = [
    0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x00,
];
const OTHER_EPOCH: [u8; 16] = [0x77; 16];

fn net(raw: u64) -> NetId {
    NetId::new(raw).expect("fixture id is nonzero")
}

fn ability() -> Ulid {
    Ulid::from_parts(0x0102_0304_0506, 0x0708_090A_0B0C_0D0E)
}

fn intent(epoch: [u8; 16], seq: u64, observed: u64) -> Vec<u8> {
    encode_intent(&IntentFrame {
        epoch,
        lane: LANE_COMBAT,
        seq,
        observed_tick: observed,
        actor: net(1),
        body: IntentBody::DeclareAction {
            ability: ability(),
            target: net(2),
        },
    })
    .expect("fixture intent encodes")
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

fn fabric() -> (InMemoryTransport, PeerId) {
    let mut transport = InMemoryTransport::new(generous_caps(), generous_rates(), SimClock(0));
    let peer = transport.add_peer(EPOCH).expect("peer admitted");
    (transport, peer)
}

// ---------------------------------------------------------------------------
// Mechanism units: clock, buckets, defaults, peer table.
// ---------------------------------------------------------------------------

#[test]
fn clock_advances_and_saturates() {
    let mut clock = SimClock(0);
    assert_eq!(clock.now(), 0);
    clock.advance(999);
    assert_eq!(clock.now(), 999);
    clock.advance(u64::MAX);
    assert_eq!(clock.now(), u64::MAX, "saturates rather than wraps");
}

#[test]
fn token_bucket_replenishes_exactly() {
    let mut bucket = TokenBucket::new(2, 2);
    assert!(bucket.consume(0, 2));
    assert!(!bucket.consume(0, 1), "empty bucket refuses");
    assert_eq!(bucket.available(), 0);
    // 500 ms at 2/s credits exactly one token.
    assert!(!bucket.consume(499, 1));
    assert!(bucket.consume(500, 1));
    assert!(!bucket.consume(500, 1));
    // Stepping is exact: ten 100 ms advances equal one 1,000 ms advance.
    let mut stepped = TokenBucket::new(2, 2);
    assert!(stepped.consume(0, 2));
    for step in 1..=10 {
        stepped.consume(step * 100, 0);
    }
    let mut jumped = TokenBucket::new(2, 2);
    assert!(jumped.consume(0, 2));
    jumped.consume(1_000, 0);
    assert_eq!(stepped.available(), jumped.available());
    assert_eq!(jumped.available(), 2, "capped at burst, never over");
    // Backwards timestamps accrue nothing and never go negative.
    assert!(stepped.consume(10, 2));
    assert!(!stepped.consume(10, 1));
}

#[test]
fn v1_defaults_match_policy() {
    let caps = QueueCaps::v1();
    assert_eq!(caps.per_peer_frames, POLICY_INGRESS_FRAMES_PER_PEER);
    assert_eq!(caps.per_peer_bytes, POLICY_INGRESS_BYTES_PER_PEER);
    assert_eq!(caps.host_frames, POLICY_INGRESS_FRAMES_HOST);
    assert_eq!(caps.host_bytes, POLICY_INGRESS_BYTES_HOST);
    let egress = QueueCaps::v1_egress();
    assert_eq!(egress.per_peer_frames, POLICY_EGRESS_FRAMES_PER_PEER);
    assert_eq!(egress.per_peer_bytes, POLICY_EGRESS_BYTES_PER_PEER);
    assert_eq!(egress.host_bytes, POLICY_EGRESS_BYTES_HOST);
    let rates = RateCaps::v1();
    assert_eq!(rates.frames_per_sec, POLICY_INTENT_FRAMES_PER_SEC as u32);
    assert_eq!(rates.burst_frames, POLICY_INTENT_FRAME_BURST as u32);
    assert_eq!(rates.bytes_per_sec, POLICY_INTENT_BYTES_PER_SEC as u32);
    assert_eq!(rates.burst_bytes, POLICY_INTENT_BYTE_BURST as u32);
    assert_eq!(rates.fail_per_sec, POLICY_FAIL_FRAMES_PER_SEC as u32);
    assert_eq!(rates.burst_fail, POLICY_FAIL_FRAME_BURST as u32);
    assert_eq!(rates.fail_bytes_per_sec, POLICY_FAIL_BYTES_PER_SEC as u32);
    assert_eq!(rates.burst_fail_bytes, POLICY_FAIL_BYTE_BURST as u32);
    let state = PeerState::new(EPOCH, &rates);
    assert_eq!(state.epoch, EPOCH);
    assert_eq!(state.lane, LANE_COMBAT);
    assert_eq!(state.next_new_seq, SEQ_FIRST);
    assert!(state.cache.is_empty());
    assert_eq!(state.fail_budget.available(), POLICY_FAIL_FRAME_BURST);
    assert_eq!(state.byte_budget.available(), POLICY_FAIL_BYTE_BURST);
    assert_eq!(POLICY_PROOF_PEERS, 8);
    assert_eq!(POLICY_RETRY_CACHE_PER_PEER_LANE, 256);
    assert_eq!(POLICY_RETRY_CACHE_BYTES_PER_PEER_LANE, 2 * 1024 * 1024);
    assert_eq!(POLICY_SNAPSHOT_IN_FLIGHT_PER_PEER, 1);
    assert_eq!(POLICY_SNAPSHOT_REASSEMBLY_SECS, 5);
    assert_eq!(POLICY_OBSERVED_TICK_WINDOW, 200);
}

#[test]
fn peer_table_bounded_and_ids_not_reused() {
    let mut transport = InMemoryTransport::new(generous_caps(), generous_rates(), SimClock(0));
    let mut ids = Vec::new();
    for _ in 0..POLICY_PROOF_PEERS {
        ids.push(transport.add_peer(EPOCH).expect("proof population fits"));
    }
    assert_eq!(
        transport.add_peer(EPOCH),
        Err(RejectionCode::ServerBusy),
        "ninth peer refused"
    );
    transport.remove_peer(ids[3]);
    let fresh = transport.add_peer(EPOCH).expect("slot freed");
    assert!(!ids.contains(&fresh), "ids never reused");
    assert_eq!(
        transport.send_to(ids[3], vec![0x01]),
        Err(RejectionCode::Unauthenticated),
        "removed peer has no binding"
    );
    assert_eq!(transport.recv_from(ids[3]), None);
    transport.remove_peer(ids[3]); // unknown removal is a no-op
}

// ---------------------------------------------------------------------------
// 1. Backpressure: frame AND byte ceilings, per-peer AND host.
// ---------------------------------------------------------------------------

#[test]
fn backpressure_per_peer_frame_and_byte_caps() {
    let caps = QueueCaps {
        per_peer_frames: 2,
        per_peer_bytes: 200,
        host_frames: 1_000,
        host_bytes: 1_000_000,
    };
    let mut transport = InMemoryTransport::new(caps, generous_rates(), SimClock(0));
    let peer = transport.add_peer(EPOCH).expect("peer admitted");
    assert!(transport.send_to(peer, vec![0xAA; 10]).is_ok());
    assert!(transport.send_to(peer, vec![0xBB; 10]).is_ok());
    assert_eq!(
        transport.send_to(peer, vec![0xCC; 10]),
        Err(RejectionCode::QueueFull),
        "third frame past the per-peer frame cap"
    );
    // Nothing grew: exactly the two admitted datagrams deliver.
    transport.tick(&FaultSchedule::clean());
    assert!(transport.recv_from(peer).is_some());
    assert!(transport.recv_from(peer).is_some());
    assert_eq!(transport.recv_from(peer), None);

    let mut transport = InMemoryTransport::new(caps, generous_rates(), SimClock(0));
    let peer = transport.add_peer(EPOCH).expect("peer admitted");
    assert!(transport.send_to(peer, vec![0xAA; 150]).is_ok());
    assert_eq!(
        transport.send_to(peer, vec![0xBB; 150]),
        Err(RejectionCode::QueueFull),
        "300 bytes past the 200-byte per-peer cap"
    );
    transport.tick(&FaultSchedule::clean());
    assert_eq!(transport.recv_from(peer).expect("one delivers").len(), 150);
    assert_eq!(transport.recv_from(peer), None);
}

#[test]
fn backpressure_host_caps_across_peers() {
    let caps = QueueCaps {
        per_peer_frames: 1_000,
        per_peer_bytes: 1_000_000,
        host_frames: 3,
        host_bytes: 300,
    };
    let mut transport = InMemoryTransport::new(caps, generous_rates(), SimClock(0));
    let first = transport.add_peer(EPOCH).expect("peer admitted");
    let second = transport.add_peer(EPOCH).expect("peer admitted");
    assert!(transport.send_to(first, vec![0x01; 10]).is_ok());
    assert!(transport.send_to(first, vec![0x02; 10]).is_ok());
    assert!(transport.send_to(second, vec![0x03; 10]).is_ok());
    assert_eq!(
        transport.send_to(second, vec![0x04; 10]),
        Err(RejectionCode::QueueFull),
        "fourth frame past the host frame cap"
    );
    assert_eq!(
        transport.send_to(first, vec![0x05; 271]),
        Err(RejectionCode::QueueFull),
        "301 host bytes past the 300-byte host cap"
    );
    // Removing a peer releases host accounting: room returns.
    transport.remove_peer(first);
    assert!(transport.send_to(second, vec![0x06; 10]).is_ok());
}

// ---------------------------------------------------------------------------
// 2. Rate: buckets gate, invalids count, injected time replenishes.
// ---------------------------------------------------------------------------

#[test]
fn rate_buckets_gate_and_replenish_exactly() {
    let rates = RateCaps {
        frames_per_sec: 2,
        burst_frames: 2,
        bytes_per_sec: 1_000_000,
        burst_bytes: 1_000_000,
        ..generous_rates()
    };
    let mut transport = InMemoryTransport::new(generous_caps(), rates, SimClock(0));
    let peer = transport.add_peer(EPOCH).expect("peer admitted");
    assert!(transport.send_to(peer, vec![0x01]).is_ok());
    assert!(transport.send_to(peer, vec![0x02]).is_ok());
    assert_eq!(
        transport.send_to(peer, vec![0x03]),
        Err(RejectionCode::RateLimited)
    );
    transport.clock_mut().advance(1_000);
    assert!(transport.send_to(peer, vec![0x04]).is_ok());

    let rates = RateCaps {
        frames_per_sec: 1_000_000,
        burst_frames: 1_000_000,
        bytes_per_sec: 100,
        burst_bytes: 100,
        ..generous_rates()
    };
    let mut transport = InMemoryTransport::new(generous_caps(), rates, SimClock(0));
    let peer = transport.add_peer(EPOCH).expect("peer admitted");
    assert!(transport.send_to(peer, vec![0xAA; 60]).is_ok());
    assert_eq!(
        transport.send_to(peer, vec![0xBB; 60]),
        Err(RejectionCode::RateLimited),
        "40 byte budget left cannot take 60"
    );
    transport.clock_mut().advance(1_000);
    assert!(transport.send_to(peer, vec![0xCC; 100]).is_ok());
}

#[test]
fn retries_and_invalids_consume_ingress() {
    let rates = RateCaps {
        frames_per_sec: 1_000_000,
        burst_frames: 1_000_000,
        bytes_per_sec: 70_000,
        burst_bytes: 70_000,
        ..generous_rates()
    };
    let mut transport = InMemoryTransport::new(generous_caps(), rates, SimClock(0));
    let peer = transport.add_peer(EPOCH).expect("peer admitted");
    // Garbage still burns budget: burst of 1 frame, garbage first.
    let tight = RateCaps {
        burst_frames: 1,
        ..generous_rates()
    };
    let mut tight_transport = InMemoryTransport::new(generous_caps(), tight, SimClock(0));
    let tight_peer = tight_transport.add_peer(EPOCH).expect("peer admitted");
    assert!(tight_transport.send_to(tight_peer, vec![0xFF; 4]).is_ok());
    assert_eq!(
        tight_transport.send_to(tight_peer, intent(EPOCH, 1, 0)),
        Err(RejectionCode::RateLimited),
        "the earlier invalid attempt consumed the single frame token"
    );
    // Oversized attempts consume too, then fail closed on size.
    let oversized = vec![0xAA; MAX_DELTA_FRAME_BYTES + 1];
    assert_eq!(
        transport.send_to(peer, oversized),
        Err(RejectionCode::FrameTooLarge)
    );
    assert_eq!(
        transport.send_to(peer, vec![0xBB; 5_000]),
        Err(RejectionCode::RateLimited),
        "65,537-byte attempt left only 4,463 of 70,000"
    );
    // With an empty budget the rate gate fires before the size gate.
    assert_eq!(
        transport.send_to(peer, vec![0xCC; MAX_DELTA_FRAME_BYTES + 1]),
        Err(RejectionCode::RateLimited)
    );
}

// ---------------------------------------------------------------------------
// Canned test-double host: epoch + seq/cache + freshness, no sim.
// ---------------------------------------------------------------------------

fn map_codec(error: CodecError) -> RejectionCode {
    match error {
        CodecError::FrameTooLarge => RejectionCode::FrameTooLarge,
        CodecError::UnsupportedVersion => RejectionCode::UnsupportedVersion,
        CodecError::WrongDirection => RejectionCode::WrongDirection,
        CodecError::UnknownMessage => RejectionCode::UnknownMessage,
        CodecError::Malformed => RejectionCode::Malformed,
        CodecError::LimitExceeded => RejectionCode::LimitExceeded,
    }
}

/// Net-local host stand-in: admits decoded intents through epoch, per-lane
/// seq/cache, and freshness checks with canned legality (`NIL` ability is
/// the unknown-ability stand-in; everything else applies). It proves the
/// net/host boundary separation; passing here is not host integration
/// passing, and T018c replaces the canned legality with real sim calls.
struct HostDouble {
    states: BTreeMap<PeerId, PeerState>,
    closed: BTreeSet<[u8; 16]>,
    delivery: BTreeMap<PeerId, u64>,
    server_tick: u64,
    executions: u64,
    fail_now_ms: u64,
}

impl HostDouble {
    fn new(server_tick: u64) -> Self {
        Self {
            states: BTreeMap::new(),
            closed: BTreeSet::new(),
            delivery: BTreeMap::new(),
            server_tick,
            executions: 0,
            fail_now_ms: 0,
        }
    }

    fn bind(&mut self, peer: PeerId, epoch: [u8; 16], rates: &RateCaps) {
        self.states.insert(peer, PeerState::new(epoch, rates));
    }

    fn advance_fail_time(&mut self, dt_ms: u64) {
        self.fail_now_ms = self.fail_now_ms.saturating_add(dt_ms);
    }

    fn test_set_next_seq(&mut self, peer: PeerId, seq: u64) {
        if let Some(state) = self.states.get_mut(&peer) {
            state.next_new_seq = seq;
        }
    }

    fn admit(&mut self, peer: PeerId, bytes: &[u8]) -> ReceiptStatus {
        let frame = match decode_intent(bytes) {
            Err(error) => return ReceiptStatus::Rejected(map_codec(error)),
            Ok(frame) => frame,
        };
        let state = match self.states.get_mut(&peer) {
            None => return ReceiptStatus::Rejected(RejectionCode::Unauthenticated),
            Some(state) => state,
        };
        if self.closed.contains(&frame.epoch) || frame.epoch != state.epoch {
            return ReceiptStatus::Rejected(RejectionCode::SessionExpired);
        }
        if let Some((_, canonical, status)) =
            state.cache.iter().find(|(seq, _, _)| *seq == frame.seq)
        {
            if canonical.as_slice() == bytes {
                return *status;
            }
            self.closed.insert(frame.epoch);
            return ReceiptStatus::Rejected(RejectionCode::SeqConflict);
        }
        if state.is_exhausted() {
            return ReceiptStatus::Rejected(RejectionCode::SeqExhausted);
        }
        if frame.seq > state.next_new_seq {
            return ReceiptStatus::Rejected(RejectionCode::SeqGap);
        }
        if frame.seq < state.next_new_seq {
            return ReceiptStatus::Rejected(RejectionCode::StaleSeq);
        }
        if frame.observed_tick > self.server_tick {
            return self.finalize(peer, &frame, bytes, RejectionCode::FutureTick);
        }
        if frame.observed_tick < self.server_tick.saturating_sub(POLICY_OBSERVED_TICK_WINDOW) {
            return self.finalize(peer, &frame, bytes, RejectionCode::StaleTick);
        }
        // Canned legality stand-in: the NIL ability is "unknown".
        let status = match frame.body {
            IntentBody::DeclareAction { ability, .. } if ability == Ulid::NIL => {
                ReceiptStatus::Rejected(RejectionCode::IllegalAction)
            }
            IntentBody::DeclareAction { .. } | IntentBody::EndTurn => {
                self.executions += 1;
                ReceiptStatus::Applied
            }
        };
        self.record(peer, &frame, bytes, status);
        status
    }

    fn finalize(
        &mut self,
        peer: PeerId,
        frame: &IntentFrame,
        bytes: &[u8],
        code: RejectionCode,
    ) -> ReceiptStatus {
        let status = ReceiptStatus::Rejected(code);
        self.record(peer, frame, bytes, status);
        status
    }

    fn record(&mut self, peer: PeerId, frame: &IntentFrame, bytes: &[u8], status: ReceiptStatus) {
        let state = self.states.get_mut(&peer).expect("peer bound");
        state.cache.push((frame.seq, bytes.to_vec(), status));
        while state.cache.len() > POLICY_RETRY_CACHE_PER_PEER_LANE {
            state.cache.remove(0);
        }
        while cache_bytes(&state.cache) > POLICY_RETRY_CACHE_BYTES_PER_PEER_LANE {
            state.cache.remove(0);
        }
        state.next_new_seq = frame.seq.saturating_add(1);
    }

    /// Encodes one receipt frame, or [`None`] when a failure response is
    /// dropped on the wire: rejected outcomes (fresh or cached) spend the
    /// peer's failure-response budgets first, and an empty budget drops the
    /// bytes while the terminal outcome stays retained for retry. Applied
    /// outcomes never touch the failure budgets.
    fn receipt_bytes(
        &mut self,
        peer: PeerId,
        frame: &IntentFrame,
        status: &ReceiptStatus,
    ) -> Option<Vec<u8>> {
        if matches!(status, ReceiptStatus::Rejected(_)) {
            let seq = self.delivery.get(&peer).copied().unwrap_or(0);
            let estimate = encode_delta(&DeltaFrame {
                lane: LANE_COMBAT,
                server_tick: self.server_tick,
                event_seq: seq,
                ops: vec![DeltaOp::Receipt {
                    epoch: frame.epoch,
                    lane: frame.lane,
                    seq: frame.seq,
                    processed_tick: self.server_tick,
                    status: *status,
                }],
            })
            .expect("receipt estimate encodes");
            let state = self.states.get_mut(&peer)?;
            if !state.try_consume_failure(self.fail_now_ms, estimate.len() as u64) {
                return None;
            }
        }
        let seq = self.delivery.get(&peer).copied().unwrap_or(0);
        self.delivery.insert(peer, seq.saturating_add(1));
        Some(
            encode_delta(&DeltaFrame {
                lane: LANE_COMBAT,
                server_tick: self.server_tick,
                event_seq: seq,
                ops: vec![DeltaOp::Receipt {
                    epoch: frame.epoch,
                    lane: frame.lane,
                    seq: frame.seq,
                    processed_tick: self.server_tick,
                    status: *status,
                }],
            })
            .expect("receipt encodes"),
        )
    }
}

fn cache_bytes(cache: &[(u64, Vec<u8>, ReceiptStatus)]) -> usize {
    cache.iter().map(|(_, bytes, _)| bytes.len()).sum()
}

// ---------------------------------------------------------------------------
// 3. Retry: cached receipts, conflict close, gaps, eviction.
// ---------------------------------------------------------------------------

#[test]
fn retry_same_bytes_returns_cached_receipt_without_reexecution() {
    let mut host = HostDouble::new(1_000);
    let (_, peer) = fabric();
    host.bind(peer, EPOCH, &generous_rates());
    let bytes = intent(EPOCH, 1, 900);
    assert_eq!(host.admit(peer, &bytes), ReceiptStatus::Applied);
    assert_eq!(host.executions, 1);
    assert_eq!(host.admit(peer, &bytes), ReceiptStatus::Applied);
    assert_eq!(host.executions, 1, "cached retry never re-executes");
    // Terminal rejections cache too.
    let illegal = encode_intent(&IntentFrame {
        epoch: EPOCH,
        lane: LANE_COMBAT,
        seq: 2,
        observed_tick: 900,
        actor: net(1),
        body: IntentBody::DeclareAction {
            ability: Ulid::NIL,
            target: net(2),
        },
    })
    .expect("encodes");
    assert_eq!(
        host.admit(peer, &illegal),
        ReceiptStatus::Rejected(RejectionCode::IllegalAction)
    );
    assert_eq!(host.executions, 1);
    assert_eq!(
        host.admit(peer, &illegal),
        ReceiptStatus::Rejected(RejectionCode::IllegalAction)
    );
    assert_eq!(host.executions, 1);
}

#[test]
fn retry_over_transport_recovers_dropped_receipt() {
    let (mut transport, peer) = fabric();
    let mut host = HostDouble::new(1_000);
    host.bind(peer, EPOCH, &generous_rates());
    let bytes = intent(EPOCH, 1, 900);

    // Client -> host, clean delivery.
    transport.send_to(peer, bytes.clone()).expect("staged");
    transport.tick(&FaultSchedule::clean());
    let arrived = transport.recv_from(peer).expect("intent delivers");
    let frame = decode_intent(&arrived).expect("decodes");
    let status = host.admit(peer, &arrived);
    assert_eq!(status, ReceiptStatus::Applied);

    // Host -> client, but the fault schedule drops the receipt.
    let receipt = host
        .receipt_bytes(peer, &frame, &status)
        .expect("applied receipt within budgets");
    transport.send_to(peer, receipt).expect("staged");
    transport.tick(&FaultSchedule {
        loss_every: Some(1),
        dup_every: None,
        reorder_depth: 0,
        seed: 0,
    });
    assert_eq!(transport.recv_from(peer), None, "receipt dropped");

    // Retry of the same canonical bytes recovers the cached receipt.
    transport.send_to(peer, bytes.clone()).expect("staged");
    transport.tick(&FaultSchedule::clean());
    let arrived = transport.recv_from(peer).expect("retry delivers");
    assert_eq!(host.admit(peer, &arrived), ReceiptStatus::Applied);
    assert_eq!(host.executions, 1, "no second execution");
    let frame = decode_intent(&arrived).expect("decodes");
    let receipt = host
        .receipt_bytes(peer, &frame, &ReceiptStatus::Applied)
        .expect("applied receipt within budgets");
    transport.send_to(peer, receipt).expect("staged");
    transport.tick(&FaultSchedule::clean());
    let delivered = transport.recv_from(peer).expect("receipt delivers");
    match decode_delta(&delivered).expect("decodes").ops.as_slice() {
        [DeltaOp::Receipt {
            seq: 1,
            status: ReceiptStatus::Applied,
            ..
        }] => {}
        other => panic!("unexpected receipt ops: {other:?}"),
    }
}

#[test]
fn seq_conflict_closes_epoch_gap_does_not_advance() {
    let mut host = HostDouble::new(1_000);
    let (_, peer) = fabric();
    host.bind(peer, EPOCH, &generous_rates());
    let first = intent(EPOCH, 1, 900);
    assert_eq!(host.admit(peer, &first), ReceiptStatus::Applied);
    // Same seq, different canonical bytes: conflict closes the epoch.
    let conflict = intent(EPOCH, 1, 901);
    assert_ne!(conflict, first);
    assert_eq!(
        host.admit(peer, &conflict),
        ReceiptStatus::Rejected(RejectionCode::SeqConflict)
    );
    assert_eq!(host.executions, 1);
    // The epoch is closed: even the next new seq is refused.
    assert_eq!(
        host.admit(peer, &intent(EPOCH, 2, 900)),
        ReceiptStatus::Rejected(RejectionCode::SessionExpired)
    );
}

#[test]
fn seq_gap_leaves_next_new_seq_alone() {
    let mut host = HostDouble::new(1_000);
    let (_, peer) = fabric();
    host.bind(peer, EPOCH, &generous_rates());
    assert_eq!(
        host.admit(peer, &intent(EPOCH, 1, 900)),
        ReceiptStatus::Applied
    );
    assert_eq!(
        host.admit(peer, &intent(EPOCH, 3, 900)),
        ReceiptStatus::Rejected(RejectionCode::SeqGap)
    );
    assert_eq!(host.executions, 1);
    assert_eq!(
        host.admit(peer, &intent(EPOCH, 2, 900)),
        ReceiptStatus::Applied
    );
    assert_eq!(host.executions, 2);
}

#[test]
fn evicted_seq_is_stale() {
    let mut host = HostDouble::new(1_000);
    let (_, peer) = fabric();
    host.bind(peer, EPOCH, &generous_rates());
    let first = intent(EPOCH, 1, 900);
    assert_eq!(host.admit(peer, &first), ReceiptStatus::Applied);
    for seq in 2..=(POLICY_RETRY_CACHE_PER_PEER_LANE as u64 + 1) {
        assert_eq!(
            host.admit(peer, &intent(EPOCH, seq, 900)),
            ReceiptStatus::Applied
        );
    }
    assert_eq!(
        host.admit(peer, &first),
        ReceiptStatus::Rejected(RejectionCode::StaleSeq),
        "seq 1 fell out of the 256-entry cache"
    );
    assert_eq!(host.executions, POLICY_RETRY_CACHE_PER_PEER_LANE as u64 + 1);
}

#[test]
fn exhausted_lane_signals_new_epoch_not_stale() {
    let mut host = HostDouble::new(1_000);
    let (_, peer) = fabric();
    host.bind(peer, EPOCH, &generous_rates());
    host.test_set_next_seq(peer, SEQ_LAST);
    let last = intent(EPOCH, SEQ_LAST, 900);
    assert_eq!(
        host.admit(peer, &last),
        ReceiptStatus::Applied,
        "last issuable seq finalizes"
    );
    // Cached retry still recovers the retained outcome after exhaustion.
    assert_eq!(
        host.admit(peer, &last),
        ReceiptStatus::Applied,
        "cached retry survives exhaustion"
    );
    // Any other non-cached seq now reports exhaustion, never a retryable gap.
    assert_eq!(
        host.admit(peer, &intent(EPOCH, 1, 900)),
        ReceiptStatus::Rejected(RejectionCode::SeqExhausted),
        "exhausted lane needs a new epoch"
    );
    assert_eq!(host.executions, 1, "exhaustion consumes no execution");
}

#[test]
fn failure_budgets_gate_receipt_bytes_and_replenish() {
    let tight = RateCaps {
        fail_per_sec: 2,
        burst_fail: 2,
        fail_bytes_per_sec: 1_000_000,
        burst_fail_bytes: 1_000_000,
        ..generous_rates()
    };
    let mut host = HostDouble::new(1_000);
    let (_, peer) = fabric();
    host.bind(peer, EPOCH, &tight);
    let illegal = |seq: u64| {
        encode_intent(&IntentFrame {
            epoch: EPOCH,
            lane: LANE_COMBAT,
            seq,
            observed_tick: 900,
            actor: net(1),
            body: IntentBody::DeclareAction {
                ability: Ulid::NIL,
                target: net(2),
            },
        })
        .expect("encodes")
    };
    for seq in 1..=2 {
        let bytes = illegal(seq);
        assert_eq!(
            host.admit(peer, &bytes),
            ReceiptStatus::Rejected(RejectionCode::IllegalAction)
        );
        let frame = decode_intent(&bytes).expect("decodes");
        assert!(
            host.receipt_bytes(
                peer,
                &frame,
                &ReceiptStatus::Rejected(RejectionCode::IllegalAction)
            )
            .is_some(),
            "burst covers seq {seq}"
        );
    }
    // Burst spent: the next failure is retained but its bytes drop.
    let bytes = illegal(3);
    assert_eq!(
        host.admit(peer, &bytes),
        ReceiptStatus::Rejected(RejectionCode::IllegalAction)
    );
    let frame = decode_intent(&bytes).expect("decodes");
    assert_eq!(
        host.receipt_bytes(
            peer,
            &frame,
            &ReceiptStatus::Rejected(RejectionCode::IllegalAction)
        ),
        None,
        "empty frame budget drops the response"
    );
    // The outcome stays retained: the status path still reports it.
    assert_eq!(
        host.admit(peer, &bytes),
        ReceiptStatus::Rejected(RejectionCode::IllegalAction)
    );
    host.advance_fail_time(1_000);
    assert!(
        host.receipt_bytes(
            peer,
            &frame,
            &ReceiptStatus::Rejected(RejectionCode::IllegalAction)
        )
        .is_some(),
        "injected second replenishes exactly"
    );
    // Applied receipts never touch the failure budgets.
    let applied = intent(EPOCH, 4, 900);
    assert_eq!(host.admit(peer, &applied), ReceiptStatus::Applied);
    let frame = decode_intent(&applied).expect("decodes");
    assert!(
        host.receipt_bytes(peer, &frame, &ReceiptStatus::Applied)
            .is_some(),
        "applied bypasses failure budgets"
    );
}

#[test]
fn failure_byte_budget_gates_independently() {
    let tiny_bytes = RateCaps {
        fail_per_sec: 1_000_000,
        burst_fail: 1_000_000,
        fail_bytes_per_sec: 1_000_000,
        burst_fail_bytes: 10,
        ..generous_rates()
    };
    let mut host = HostDouble::new(1_000);
    let (_, peer) = fabric();
    host.bind(peer, EPOCH, &tiny_bytes);
    let bytes = intent(EPOCH, 1, 900);
    assert_eq!(host.admit(peer, &bytes), ReceiptStatus::Applied);
    // A ~30-byte receipt cannot fit a 10-byte burst: dropped, not partial.
    let frame = decode_intent(&bytes).expect("decodes");
    let dropped = host.receipt_bytes(
        peer,
        &frame,
        &ReceiptStatus::Rejected(RejectionCode::IllegalAction),
    );
    assert_eq!(dropped, None, "byte budget gates before any frame");
}

#[test]
fn egress_v1_caps_fail_closed() {
    // Egress enforcement point is fabric staging with the downstream depths:
    // drivers stage host-to-peer bytes through the same rate → size → queue
    // checks, so a slow peer terminates with QueueFull, never silent loss.
    let mut transport =
        InMemoryTransport::new(QueueCaps::v1_egress(), generous_rates(), SimClock(0));
    let peer = transport.add_peer(EPOCH).expect("peer admitted");
    for _ in 0..POLICY_EGRESS_FRAMES_PER_PEER {
        transport.send_to(peer, vec![0xAA]).expect("fits");
    }
    assert_eq!(
        transport.send_to(peer, vec![0xBB]),
        Err(RejectionCode::QueueFull),
        "129th frame past the 128-frame egress cap"
    );
    transport.tick(&FaultSchedule::clean());
    let mut drained = 0;
    while transport.recv_from(peer).is_some() {
        drained += 1;
    }
    assert_eq!(drained, POLICY_EGRESS_FRAMES_PER_PEER);

    let mut transport =
        InMemoryTransport::new(QueueCaps::v1_egress(), generous_rates(), SimClock(0));
    let peer = transport.add_peer(EPOCH).expect("peer admitted");
    // Single datagrams cap at the 64 KiB fabric MTU; the 2 MiB egress budget
    // is total staged bytes, so fill it with 32 full-size datagrams.
    for _ in 0..32 {
        transport
            .send_to(peer, vec![0xAA; MAX_DELTA_FRAME_BYTES])
            .expect("64 KiB fits the MTU");
    }
    assert_eq!(
        transport.send_to(peer, vec![0xBB]),
        Err(RejectionCode::QueueFull),
        "one byte past the 2-MiB egress cap"
    );
    assert_eq!(
        POLICY_EGRESS_BYTES_HOST,
        16 * 1024 * 1024,
        "host egress ceiling stays pinned"
    );
}

// ---------------------------------------------------------------------------
// 4. Fault determinism: seeded reorder bounded, loss/dup periodic.
// ---------------------------------------------------------------------------

fn addressed(count: usize) -> Vec<Vec<u8>> {
    (1..=count).map(|i| vec![i as u8]).collect()
}

fn deliver_all(
    transport: &mut InMemoryTransport,
    peer: PeerId,
    faults: &FaultSchedule,
) -> Vec<Vec<u8>> {
    transport.tick(faults);
    let mut out = Vec::new();
    while let Some(bytes) = transport.recv_from(peer) {
        out.push(bytes);
    }
    out
}

#[test]
fn fault_schedule_is_deterministic_per_seed() {
    let faults = FaultSchedule {
        loss_every: None,
        dup_every: None,
        reorder_depth: 3,
        seed: 7,
    };
    let run = || {
        let (mut transport, peer) = fabric();
        for bytes in addressed(12) {
            transport.send_to(peer, bytes).expect("staged");
        }
        deliver_all(&mut transport, peer, &faults)
    };
    let first = run();
    let second = run();
    assert_eq!(first, second, "same seed and sequence, same delivery");
    assert_eq!(first.len(), 12);
    assert_ne!(first, addressed(12), "seed 7 actually reorders this batch");
    // Reorder displacement stays within the depth window.
    for (position, bytes) in first.iter().enumerate() {
        let original = (bytes[0] - 1) as usize;
        let displacement = position.abs_diff(original);
        assert!(
            displacement <= 3,
            "datagram {} moved {displacement} places",
            bytes[0]
        );
    }
    // Clean delivery preserves order exactly.
    let (mut transport, peer) = fabric();
    for bytes in addressed(6) {
        transport.send_to(peer, bytes).expect("staged");
    }
    assert_eq!(
        deliver_all(&mut transport, peer, &FaultSchedule::clean()),
        addressed(6)
    );
}

#[test]
fn loss_and_dup_are_periodic() {
    let (mut transport, peer) = fabric();
    for bytes in addressed(9) {
        transport.send_to(peer, bytes).expect("staged");
    }
    assert_eq!(
        deliver_all(
            &mut transport,
            peer,
            &FaultSchedule {
                loss_every: Some(3),
                dup_every: None,
                reorder_depth: 0,
                seed: 0,
            }
        ),
        vec![vec![1], vec![2], vec![4], vec![5], vec![7], vec![8]],
        "every 3rd datagram dropped"
    );
    let (mut transport, peer) = fabric();
    for bytes in addressed(4) {
        transport.send_to(peer, bytes).expect("staged");
    }
    assert_eq!(
        deliver_all(
            &mut transport,
            peer,
            &FaultSchedule {
                loss_every: None,
                dup_every: Some(2),
                reorder_depth: 0,
                seed: 0,
            }
        ),
        vec![vec![1], vec![2], vec![2], vec![3], vec![4], vec![4]],
        "every 2nd datagram doubled in place"
    );
}

#[test]
fn multi_peer_pump_is_deterministic() {
    let faults = FaultSchedule {
        loss_every: None,
        dup_every: None,
        reorder_depth: 2,
        seed: 11,
    };
    let run = || {
        let mut transport = InMemoryTransport::new(generous_caps(), generous_rates(), SimClock(0));
        let first = transport.add_peer(EPOCH).expect("peer admitted");
        let second = transport.add_peer(OTHER_EPOCH).expect("peer admitted");
        for i in 1..=6u8 {
            transport.send_to(first, vec![0xA0, i]).expect("staged");
            transport.send_to(second, vec![0xB0, i]).expect("staged");
        }
        transport.tick(&faults);
        let mut out = Vec::new();
        while let Some(bytes) = transport.recv_from(first) {
            out.push(bytes);
        }
        while let Some(bytes) = transport.recv_from(second) {
            out.push(bytes);
        }
        out
    };
    assert_eq!(run(), run());
}

// ---------------------------------------------------------------------------
// 5. Freshness: 200-tick window with saturating edges.
// ---------------------------------------------------------------------------

#[test]
fn freshness_window_boundaries() {
    let mut host = HostDouble::new(1_000);
    let (_, peer) = fabric();
    host.bind(peer, EPOCH, &generous_rates());
    // Boundary: 200 old accepted, 201 old rejected, future rejected.
    let mut seq = 1;
    let mut admit_at = |host: &mut HostDouble, observed: u64| {
        let status = host.admit(peer, &intent(EPOCH, seq, observed));
        seq += 1;
        status
    };
    assert_eq!(admit_at(&mut host, 800), ReceiptStatus::Applied);
    assert_eq!(
        admit_at(&mut host, 799),
        ReceiptStatus::Rejected(RejectionCode::StaleTick)
    );
    assert_eq!(
        admit_at(&mut host, 1_001),
        ReceiptStatus::Rejected(RejectionCode::FutureTick)
    );
    assert_eq!(
        admit_at(&mut host, u64::MAX),
        ReceiptStatus::Rejected(RejectionCode::FutureTick)
    );
    // Saturating edge: at server tick 0 nothing is stale.
    let mut early = HostDouble::new(0);
    early.bind(peer, EPOCH, &generous_rates());
    assert_eq!(
        early.admit(peer, &intent(EPOCH, 1, 0)),
        ReceiptStatus::Applied
    );
    assert_eq!(
        early.admit(peer, &intent(EPOCH, 2, 1)),
        ReceiptStatus::Rejected(RejectionCode::FutureTick)
    );
}

// ---------------------------------------------------------------------------
// Double plumbing: unknown bindings, codec mapping, receipt shape.
// ---------------------------------------------------------------------------

#[test]
fn unknown_peer_and_epoch_rejected() {
    let mut host = HostDouble::new(1_000);
    // Two peers on one fabric; only the first is bound, so the second reads
    // as unauthenticated without inventing a peer-id constructor.
    let mut transport = InMemoryTransport::new(generous_caps(), generous_rates(), SimClock(0));
    let peer = transport.add_peer(EPOCH).expect("peer admitted");
    let stranger = transport.add_peer(EPOCH).expect("peer admitted");
    host.bind(peer, EPOCH, &generous_rates());
    assert_eq!(
        host.admit(stranger, &intent(EPOCH, 1, 900)),
        ReceiptStatus::Rejected(RejectionCode::Unauthenticated)
    );
    assert_eq!(
        host.admit(peer, &intent(OTHER_EPOCH, 1, 900)),
        ReceiptStatus::Rejected(RejectionCode::SessionExpired)
    );
}

#[test]
fn malformed_intent_maps_to_receipt_code() {
    let mut host = HostDouble::new(1_000);
    let (_, peer) = fabric();
    host.bind(peer, EPOCH, &generous_rates());
    assert_eq!(
        host.admit(peer, &[0x01, 0x02]),
        ReceiptStatus::Rejected(RejectionCode::Malformed)
    );
    let mut bad_version = intent(EPOCH, 1, 900);
    bad_version[0] = 0x7F;
    assert_eq!(
        host.admit(peer, &bad_version),
        ReceiptStatus::Rejected(RejectionCode::UnsupportedVersion)
    );
    assert_eq!(
        host.admit(peer, &vec![0xAA; 4_097]),
        ReceiptStatus::Rejected(RejectionCode::FrameTooLarge)
    );
}

#[test]
fn receipt_bytes_round_trip_through_codec() {
    let mut host = HostDouble::new(1_000);
    let (_, peer) = fabric();
    host.bind(peer, EPOCH, &generous_rates());
    let bytes = intent(EPOCH, 1, 950);
    let status = host.admit(peer, &bytes);
    let frame = decode_intent(&bytes).expect("decodes");
    let receipt = host
        .receipt_bytes(peer, &frame, &status)
        .expect("applied receipt within budgets");
    match decode_delta(&receipt).expect("decodes").ops.as_slice() {
        [DeltaOp::Receipt {
            epoch,
            lane,
            seq: 1,
            processed_tick: 1_000,
            status: ReceiptStatus::Applied,
        }] => {
            assert_eq!(*epoch, EPOCH);
            assert_eq!(*lane, LANE_COMBAT);
        }
        other => panic!("unexpected receipt ops: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Snapshot reassembly (provisional: no v1 snapshot wire; the bound model).
// ---------------------------------------------------------------------------

/// Test-local reassembly slot proving the T018a snapshot policy composes
/// with injected time: 1 in flight, at most 32 chunks, 5 s deadline, then
/// `SnapshotExpired` and full resync. The transfer wire itself arrives with
/// a later version; this pins the policy the wire will inherit.
struct SnapshotSlot {
    chunks: usize,
    deadline_ms: u64,
}

impl SnapshotSlot {
    fn begin(now_ms: u64) -> Self {
        Self {
            chunks: 0,
            deadline_ms: now_ms.saturating_add(POLICY_SNAPSHOT_REASSEMBLY_SECS * 1_000),
        }
    }

    fn add_chunk(&mut self) -> Result<(), RejectionCode> {
        if self.chunks >= MAX_SNAPSHOT_CHUNKS {
            return Err(RejectionCode::LimitExceeded);
        }
        self.chunks += 1;
        Ok(())
    }

    fn poll(&self, now_ms: u64) -> Result<(), RejectionCode> {
        if now_ms >= self.deadline_ms {
            return Err(RejectionCode::SnapshotExpired);
        }
        Ok(())
    }
}

/// A second concurrent assembly is refused while one is in flight.
fn try_begin(slot: &Option<SnapshotSlot>, now_ms: u64) -> Result<SnapshotSlot, RejectionCode> {
    if slot.is_some() {
        return Err(RejectionCode::LimitExceeded);
    }
    Ok(SnapshotSlot::begin(now_ms))
}

#[test]
fn snapshot_reassembly_bounds() {
    let mut clock = SimClock(0);
    let mut slot: Option<SnapshotSlot> = None;
    slot = Some(try_begin(&slot, clock.now()).expect("first assembly begins"));
    assert_eq!(
        try_begin(&slot, clock.now()).map(|_| ()),
        Err(RejectionCode::LimitExceeded),
        "one in-flight snapshot per peer"
    );
    let active = slot.as_mut().expect("in flight");
    for _ in 0..MAX_SNAPSHOT_CHUNKS {
        active.add_chunk().expect("32 chunks fit");
    }
    assert_eq!(active.add_chunk(), Err(RejectionCode::LimitExceeded));
    clock.advance(4_999);
    assert!(active.poll(clock.now()).is_ok());
    clock.advance(1);
    assert_eq!(
        active.poll(clock.now()),
        Err(RejectionCode::SnapshotExpired)
    );
    slot = None;
    slot = Some(try_begin(&slot, clock.now()).expect("expiry resyncs: slot released"));
    assert!(slot.is_some());
}
