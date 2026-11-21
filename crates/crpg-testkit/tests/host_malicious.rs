//! T041c acceptance suite (`tasks/T041.md`, M1–M10): the host-level
//! malicious-client table over the production v4 wire and `combat_basic`
//! campaign data.
//!
//! Reading 12: the cases run on `Host` directly, because `LocalHost` discards
//! the pump summary and a decode failure or an ingress refusal sends no
//! receipt, so only `IngestDisposition` and `AdmissionResult` show the clean
//! rejection. Every case builds B-21's rig afresh, asserts the exact
//! disposition or admission result, the unchanged authority, the still-bound
//! session (M9 aside), and a control that flips one input and is applied.
//! Expected values come from the contract, never from running the code.

#[path = "support/headless.rs"]
mod headless;

use headless::*;

use crpg_core::EntityId;
use crpg_net::codec_v4;
use crpg_net::pool_v4::{GateMode, ReassemblyPoolV4, ReplicaUpdateV4, TransferLimitsV3};
use crpg_net::protocol::{NetId, ReceiptStatus, RejectionCode, LANE_COMBAT};
use crpg_net::protocol_transfer::TransferUp;
use crpg_net::protocol_v3::{ReceiptStatus as ReceiptV3, RejectionCode as CodeV3};
use crpg_net::protocol_v4::{IntentBody, IntentFrame, SessionEpoch};
use crpg_net::replica_v4::ReplicaWorldV4;
use crpg_net::transfer_v3;
use crpg_server::host::{
    AdmissionResult, ControlGrant, DeliveryKind, DisclosureGrants, EntityDisclosure, FactGrants,
    Host, HostError, IngestDisposition, PeerHandle, TransferDisposition,
};
use crpg_sim::{CombatError, EncounterMode};

// ---------------------------------------------------------------------------
// B-21: the rig.
// ---------------------------------------------------------------------------

/// One B-21 rig: the started host, its two combatants and the three peers.
struct Rig {
    host: Host,
    hero: EntityId,
    goblin: EntityId,
    a: PeerHandle,
    b: PeerHandle,
    p: PeerHandle,
}

/// The public `ControlGrant` over `actors`.
fn control(actors: &[EntityId]) -> ControlGrant {
    ControlGrant {
        actors: actors.to_vec(),
    }
}

/// One entity's disclosure: `present` and `health` always, every other flag
/// only when `all`.
fn disclose(entity: EntityId, all: bool) -> EntityDisclosure {
    EntityDisclosure {
        entity,
        present: true,
        health: true,
        spawn: all,
        despawn: all,
        died: all,
        action_actor: all,
        action_target: all,
        action_outcome: all,
        action_damage: all,
        turn_start: all,
    }
}

/// T037e's `bind_client` steps (`crates/crpg-testkit/tests/host_movement.rs`
/// `:572-582`): drain the snapshot, apply it, and publish its transfer id.
fn bind_client(host: &mut Host, peer: PeerHandle, now: u64) {
    let epoch = host.epoch(peer).expect("bound");
    let mut pool = ReassemblyPoolV4::new(TransferLimitsV3::v3()).expect("v3 limits");
    let rid = pool
        .add_receiver(epoch, GateMode::AwaitSnapshot)
        .expect("receiver");
    let mut replica = ReplicaWorldV4::new(SessionEpoch(epoch));
    let entries = host.take_delivery_entries(peer).expect("bound");
    for entry in &entries {
        match entry.kind {
            DeliveryKind::Transfer => {
                pool.offer_transfer(rid, &entry.bytes, now)
                    .expect("the gate accepts the frame");
            }
            DeliveryKind::Delta => {
                pool.offer_delta(rid, &entry.bytes, now)
                    .expect("the gate accepts the frame");
            }
        }
    }
    if let Some(last) = entries.last() {
        host.acknowledge_delivery(peer, last.ordinal).expect("acks");
    }
    let mut transfer = None;
    let mut deltas = 0usize;
    while let Some(update) = pool.pop_update(rid) {
        match update {
            ReplicaUpdateV4::Snapshot(publication) => {
                transfer = Some(publication.transfer);
                replica.apply_snapshot(&publication).expect("applies");
            }
            ReplicaUpdateV4::Delta(frame) => {
                replica.apply_delta(&frame).expect("applies");
                deltas += 1;
            }
        }
    }
    assert_eq!(deltas, 1, "the anchoring page");
    let bytes = transfer_v3::encode_up(&TransferUp::Published {
        epoch,
        transfer: transfer.expect("a snapshot was published"),
    })
    .expect("encodes");
    assert_eq!(
        host.ingest_transfer(peer, &bytes, now),
        Ok(TransferDisposition::Published)
    );
}

/// B-21: `start(TurnBased, None)`, then `A` (hero, `Full`), `B` (goblin,
/// `Full`) and `P` (goblin; the hero present with health only) bound in that
/// order and published. The goblin (initiative 9) holds the first turn.
fn new_rig() -> Rig {
    let mut started = start(EncounterMode::TurnBased, None);
    let hero = started.hero;
    let goblin = started.goblin;
    let full_grants = disclosure(Grants::Full, &started);
    let full_facts = fact_grants(Grants::Full, &started);
    let a = started
        .host
        .bind_v4_peer(
            control(&[hero]),
            full_grants.clone(),
            full_facts.clone(),
            NO_PAUSE,
            0,
        )
        .expect("binds A");
    let b = started
        .host
        .bind_v4_peer(
            control(&[goblin]),
            full_grants.clone(),
            full_facts.clone(),
            NO_PAUSE,
            0,
        )
        .expect("binds B");
    // P: the goblin fully disclosed, the hero present with health only.
    let p_grants = DisclosureGrants {
        entities: vec![disclose(goblin, true), disclose(hero, false)],
        abilities: vec![strike()],
        encounters: vec![first_blood()],
        turn_round: true,
        encounter_end: true,
        turn_state: true,
    };
    let p = started
        .host
        .bind_v4_peer(
            control(&[goblin]),
            p_grants,
            FactGrants::default(),
            NO_PAUSE,
            0,
        )
        .expect("binds P");
    for peer in [a, b, p] {
        assert_eq!(
            started.host.replica_id(peer, hero),
            Ok(Some(n(1))),
            "net ids"
        );
        assert_eq!(
            started.host.replica_id(peer, goblin),
            Ok(Some(n(2))),
            "net ids"
        );
    }
    for peer in [a, b, p] {
        bind_client(&mut started.host, peer, 0);
    }
    Rig {
        host: started.host,
        hero,
        goblin,
        a,
        b,
        p,
    }
}

// ---------------------------------------------------------------------------
// The three private helpers of `tasks/T041.md` §"Interface".
// ---------------------------------------------------------------------------

/// A v4 intent of `peer`'s session, encoded by `codec_v4::encode_intent`.
fn intent(
    host: &Host,
    peer: PeerHandle,
    seq: u64,
    observed: u64,
    actor: NetId,
    body: IntentBody,
) -> Vec<u8> {
    let frame = IntentFrame {
        epoch: host.epoch(peer).expect("bound"),
        lane: LANE_COMBAT,
        seq,
        observed_tick: observed,
        actor,
        body,
    };
    codec_v4::encode_intent(&frame).expect("encode")
}

/// `host.ingest(peer, bytes, now)` must be `Staged`; then one `pump(now)`; its
/// results.
fn admit(host: &mut Host, peer: PeerHandle, bytes: &[u8], now: u64) -> Vec<AdmissionResult> {
    assert_eq!(
        host.ingest(peer, bytes, now),
        Ok(IngestDisposition::Staged),
        "ingest at {now}: {}",
        hex(bytes)
    );
    host.pump(now).expect("pump").results
}

/// `(authority_hash, executions)`.
fn state(host: &Host) -> ([u8; 32], u64) {
    (host.authority_hash(), host.executions())
}

// ---------------------------------------------------------------------------
// Assertion helpers (the contract's exact values and clean-rejection shape).
// ---------------------------------------------------------------------------

/// The lowercase hex of a frame, for a failing assertion's message.
fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// `[peer, seq, Rejected(code), cached: false]`.
fn rejected(peer: PeerHandle, seq: Option<u64>, code: RejectionCode) -> AdmissionResult {
    AdmissionResult {
        peer,
        seq,
        status: ReceiptStatus::Rejected(code),
        cached: false,
        retained_in_ingress: false,
    }
}

/// `[peer, seq, Rejected(code), cached: true]`.
fn rejected_cached(peer: PeerHandle, seq: Option<u64>, code: RejectionCode) -> AdmissionResult {
    AdmissionResult {
        peer,
        seq,
        status: ReceiptStatus::Rejected(code),
        cached: true,
        retained_in_ingress: false,
    }
}

/// `[peer, seq, Applied, cached: false]`.
fn applied(peer: PeerHandle, seq: u64) -> AdmissionResult {
    AdmissionResult {
        peer,
        seq: Some(seq),
        status: ReceiptStatus::Applied,
        cached: false,
        retained_in_ingress: false,
    }
}

/// `[peer, seq, Applied, cached: true]`.
fn applied_cached(peer: PeerHandle, seq: u64) -> AdmissionResult {
    AdmissionResult {
        peer,
        seq: Some(seq),
        status: ReceiptStatus::Applied,
        cached: true,
        retained_in_ingress: false,
    }
}

/// B-22: the authority and execution count are unchanged by the hostile input.
fn assert_unchanged(before: ([u8; 32], u64), host: &Host, what: &str) {
    assert_eq!(state(host), before, "state changed by {what}");
}

/// B-22: the peer's session is still bound after the hostile input.
fn assert_bound(host: &Host, peer: PeerHandle, what: &str) {
    assert!(host.epoch(peer).is_ok(), "session unbound by {what}");
}

/// B-23: `k` advances with every delivery entry acknowledged and every sample
/// taken, so no ledger fills.
fn idle(host: &mut Host, peers: &[PeerHandle], k: u64) {
    let mut now = 0u64;
    for _ in 0..k {
        now += 50;
        let _ = host.advance(now).expect("idle");
        for &peer in peers {
            let entries = host.take_delivery_entries(peer).expect("bound");
            if let Some(last) = entries.last() {
                host.acknowledge_delivery(peer, last.ordinal).expect("acks");
            }
            let samples = host.take_samples(peer).expect("bound");
            let _ = samples.len();
        }
    }
}

/// The first index of `needle` in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

// ---------------------------------------------------------------------------
// M1-M10.
// ---------------------------------------------------------------------------

/// M1: an intent for an actor the session does not control is not authorized.
#[test]
fn m1_unowned_actor_is_not_authorized() {
    let mut rig = new_rig();
    let hostile = intent(
        &rig.host,
        rig.a,
        1,
        rig.host.server_tick(),
        n(2),
        IntentBody::EndTurn,
    );
    let before = state(&rig.host);
    let results = admit(&mut rig.host, rig.a, &hostile, 100);
    assert_eq!(
        results,
        vec![rejected(rig.a, Some(1), RejectionCode::NotAuthorized)],
        "M1 A [{}] at now 100",
        hex(&hostile)
    );
    assert_unchanged(before, &rig.host, "M1");
    assert_bound(&rig.host, rig.a, "M1");
    assert_eq!(
        rig.host.cached_status_v3(rig.a, 1),
        Ok(Some(ReceiptV3::Rejected(CodeV3::NotAuthorized))),
        "M1 A's cache after [{}]",
        hex(&hostile)
    );

    // Control: B controls the goblin, so its seq-1 EndTurn is applied.
    let control = intent(
        &rig.host,
        rig.b,
        1,
        rig.host.server_tick(),
        n(2),
        IntentBody::EndTurn,
    );
    let results = admit(&mut rig.host, rig.b, &control, 100);
    assert_eq!(
        results,
        vec![applied(rig.b, 1)],
        "M1 control [{}]",
        hex(&control)
    );
}

/// M2: an observed tick ahead of the server tick is `FutureTick`, one before
/// the 200-tick window is `StaleTick`, and the window's edge is fresh.
#[test]
fn m2_illegal_times_are_refused() {
    let mut rig = new_rig();
    let peers = [rig.a, rig.b, rig.p];
    idle(&mut rig.host, &peers, 210);
    let tick = rig.host.server_tick();
    assert_eq!(tick, 210, "210 advances tick 210 times");

    let future = intent(&rig.host, rig.b, 1, tick + 1, n(2), IntentBody::EndTurn);
    let before = state(&rig.host);
    let results = admit(&mut rig.host, rig.b, &future, 10_500);
    assert_eq!(
        results,
        vec![rejected(rig.b, Some(1), RejectionCode::FutureTick)],
        "M2 future [{}]",
        hex(&future)
    );
    assert_unchanged(before, &rig.host, "M2 future");
    assert_bound(&rig.host, rig.b, "M2 future");

    let stale = intent(&rig.host, rig.b, 2, tick - 201, n(2), IntentBody::EndTurn);
    let before = state(&rig.host);
    let results = admit(&mut rig.host, rig.b, &stale, 10_500);
    assert_eq!(
        results,
        vec![rejected(rig.b, Some(2), RejectionCode::StaleTick)],
        "M2 stale [{}]",
        hex(&stale)
    );
    assert_unchanged(before, &rig.host, "M2 stale");
    assert_bound(&rig.host, rig.b, "M2 stale");

    // Control: the newest tick at the window's lower edge is fresh.
    let edge = intent(&rig.host, rig.b, 3, tick - 200, n(2), IntentBody::EndTurn);
    let results = admit(&mut rig.host, rig.b, &edge, 10_500);
    assert_eq!(
        results,
        vec![applied(rig.b, 3)],
        "M2 control [{}]",
        hex(&edge)
    );
}

/// M3: an EndTurn by an actor that does not hold the turn is `IllegalAction`
/// and leaves sim's precedence error behind.
#[test]
fn m3_out_of_turn_is_illegal() {
    let mut rig = new_rig();
    let hostile = intent(
        &rig.host,
        rig.a,
        1,
        rig.host.server_tick(),
        n(1),
        IntentBody::EndTurn,
    );
    let before = state(&rig.host);
    let results = admit(&mut rig.host, rig.a, &hostile, 100);
    assert_eq!(
        results,
        vec![rejected(rig.a, Some(1), RejectionCode::IllegalAction)],
        "M3 A [{}]",
        hex(&hostile)
    );
    assert_unchanged(before, &rig.host, "M3");
    assert_bound(&rig.host, rig.a, "M3");
    assert_eq!(
        rig.host.last_combat_error(),
        Some(CombatError::OutOfTurn {
            actor: rig.hero,
            active: Some(rig.goblin),
        }),
        "M3's combat error"
    );

    // Control: B ends the goblin's turn, so A's seq-2 EndTurn is applied.
    let b_control = intent(
        &rig.host,
        rig.b,
        1,
        rig.host.server_tick(),
        n(2),
        IntentBody::EndTurn,
    );
    assert_eq!(
        admit(&mut rig.host, rig.b, &b_control, 100),
        vec![applied(rig.b, 1)],
        "M3 B's control [{}]",
        hex(&b_control)
    );
    let a_control = intent(
        &rig.host,
        rig.a,
        2,
        rig.host.server_tick(),
        n(1),
        IntentBody::EndTurn,
    );
    assert_eq!(
        admit(&mut rig.host, rig.a, &a_control, 100),
        vec![applied(rig.a, 2)],
        "M3 A's control [{}]",
        hex(&a_control)
    );
}

/// M4: targets outside the session's interest are not authorized, including
/// a disclosed hero with no `action_target` grant.
#[test]
fn m4_out_of_interest_targets_are_not_authorized() {
    let mut rig = new_rig();
    let unknown = intent(
        &rig.host,
        rig.b,
        1,
        rig.host.server_tick(),
        n(2),
        IntentBody::DeclareAction {
            ability: strike(),
            target: n(99),
        },
    );
    let before = state(&rig.host);
    let results = admit(&mut rig.host, rig.b, &unknown, 100);
    assert_eq!(
        results,
        vec![rejected(rig.b, Some(1), RejectionCode::NotAuthorized)],
        "M4 B unknown target [{}]",
        hex(&unknown)
    );
    assert_unchanged(before, &rig.host, "M4 B");
    assert_bound(&rig.host, rig.b, "M4 B");

    let ungranted = intent(
        &rig.host,
        rig.p,
        1,
        rig.host.server_tick(),
        n(2),
        IntentBody::DeclareAction {
            ability: strike(),
            target: n(1),
        },
    );
    let before = state(&rig.host);
    let results = admit(&mut rig.host, rig.p, &ungranted, 100);
    assert_eq!(
        results,
        vec![rejected(rig.p, Some(1), RejectionCode::NotAuthorized)],
        "M4 P [{}]",
        hex(&ungranted)
    );
    assert_unchanged(before, &rig.host, "M4 P");
    assert_bound(&rig.host, rig.p, "M4 P");

    // Control: B's seq-2 strike against the granted hero is applied.
    let control = intent(
        &rig.host,
        rig.b,
        2,
        rig.host.server_tick(),
        n(2),
        IntentBody::DeclareAction {
            ability: strike(),
            target: n(1),
        },
    );
    let results = admit(&mut rig.host, rig.b, &control, 100);
    assert_eq!(
        results,
        vec![applied(rig.b, 2)],
        "M4 control [{}]",
        hex(&control)
    );
}

/// M5: the 80-frame burst is staged, the rest is rate limited, the pump
/// rejects every staged frame as `UnsupportedVersion`, and minutes later a
/// fresh seq 1 is applied.
#[test]
fn m5_flood_is_rate_limited_then_recovers() {
    let mut rig = new_rig();
    let before = state(&rig.host);
    let flood = [0xFFu8];
    for attempt in 1..=100u64 {
        let disposition = rig.host.ingest(rig.b, &flood, 10_000);
        if attempt <= 80 {
            assert_eq!(
                disposition,
                Ok(IngestDisposition::Staged),
                "M5 flood {attempt} of [ff]"
            );
        } else {
            assert_eq!(
                disposition,
                Ok(IngestDisposition::Refused {
                    status: ReceiptStatus::Rejected(RejectionCode::RateLimited),
                }),
                "M5 flood {attempt} of [ff]"
            );
        }
    }
    let summary = rig.host.pump(10_000).expect("pump");
    assert_eq!(
        summary.results,
        vec![rejected(rig.b, None, RejectionCode::UnsupportedVersion); 80],
        "M5 pump of 80 [ff] frames"
    );
    assert_unchanged(before, &rig.host, "M5 flood");
    assert_bound(&rig.host, rig.b, "M5 flood");

    // Control: after the refill, seq 1 has never existed and is applied.
    let control = intent(&rig.host, rig.b, 1, 0, n(2), IntentBody::EndTurn);
    let results = admit(&mut rig.host, rig.b, &control, 12_000);
    assert_eq!(
        results,
        vec![applied(rig.b, 1)],
        "M5 control [{}]",
        hex(&control)
    );
}

/// M6: an oversize frame is refused before staging, a truncated frame reaches
/// admission as `Malformed`, and the whole frame is then applied.
#[test]
fn m6_oversize_and_malformed_are_clean() {
    let mut rig = new_rig();
    let oversize = vec![4u8; 4_097];
    let before = state(&rig.host);
    assert_eq!(
        rig.host.ingest(rig.b, &oversize, 100),
        Ok(IngestDisposition::Refused {
            status: ReceiptStatus::Rejected(RejectionCode::FrameTooLarge),
        }),
        "M6 oversize (4097 bytes of 04)"
    );
    let summary = rig.host.pump(100).expect("pump");
    assert_eq!(summary.results, Vec::new(), "M6 oversize staged nothing");
    assert_unchanged(before, &rig.host, "M6 oversize");
    assert_bound(&rig.host, rig.b, "M6 oversize");

    let frame = intent(
        &rig.host,
        rig.b,
        1,
        rig.host.server_tick(),
        n(2),
        IntentBody::EndTurn,
    );
    let truncated = &frame[..frame.len() - 1];
    let before = state(&rig.host);
    let results = admit(&mut rig.host, rig.b, truncated, 100);
    assert_eq!(
        results,
        vec![rejected(rig.b, None, RejectionCode::Malformed)],
        "M6 truncated [{}]",
        hex(truncated)
    );
    assert_unchanged(before, &rig.host, "M6 truncated");
    assert_bound(&rig.host, rig.b, "M6 truncated");

    // Control: the whole seq-1 frame is applied.
    let results = admit(&mut rig.host, rig.b, &frame, 100);
    assert_eq!(
        results,
        vec![applied(rig.b, 1)],
        "M6 control [{}]",
        hex(&frame)
    );
}

/// M7: the identical bytes of an applied seq 1 are a cache hit, not a second
/// execution.
#[test]
fn m7_identical_replay_is_cached() {
    let mut rig = new_rig();
    let frame = intent(
        &rig.host,
        rig.b,
        1,
        rig.host.server_tick(),
        n(2),
        IntentBody::EndTurn,
    );
    // Control: the first send is applied.
    let results = admit(&mut rig.host, rig.b, &frame, 100);
    assert_eq!(
        results,
        vec![applied(rig.b, 1)],
        "M7 first send [{}]",
        hex(&frame)
    );
    let executed = state(&rig.host);

    let results = admit(&mut rig.host, rig.b, &frame, 200);
    assert_eq!(
        results,
        vec![applied_cached(rig.b, 1)],
        "M7 replay [{}]",
        hex(&frame)
    );
    assert_unchanged(executed, &rig.host, "M7 replay");
    assert_bound(&rig.host, rig.b, "M7 replay");
}

/// M8: a seq gap does not advance the sequence; a flood of 257 future-tick
/// seqs evicts seq 1 (so its bytes are `StaleSeq`) while seq 2 stays cached.
#[test]
fn m8_gaps_and_stale_seqs_are_refused() {
    // The gap: seq 2 while seq 1 is next.
    let mut rig = new_rig();
    let gap = intent(
        &rig.host,
        rig.b,
        2,
        rig.host.server_tick(),
        n(2),
        IntentBody::EndTurn,
    );
    let before = state(&rig.host);
    let results = admit(&mut rig.host, rig.b, &gap, 100);
    assert_eq!(
        results,
        vec![rejected(rig.b, Some(2), RejectionCode::SeqGap)],
        "M8 gap [{}]",
        hex(&gap)
    );
    assert_unchanged(before, &rig.host, "M8 gap");
    assert_bound(&rig.host, rig.b, "M8 gap");
    assert_eq!(
        rig.host.cached_status_v3(rig.b, 2),
        Ok(None),
        "M8's gap left no cache entry"
    );

    // Control: the gap advanced nothing, so seq 1 is applied.
    let first = intent(
        &rig.host,
        rig.b,
        1,
        rig.host.server_tick(),
        n(2),
        IntentBody::EndTurn,
    );
    let results = admit(&mut rig.host, rig.b, &first, 100);
    assert_eq!(
        results,
        vec![applied(rig.b, 1)],
        "M8 control [{}]",
        hex(&first)
    );

    // A fresh rig: 257 future-tick seqs, then the evicted seq 1.
    let mut fresh = new_rig();
    let flood_state = state(&fresh.host);
    let mut first = Vec::new();
    let mut second = Vec::new();
    for seq in 1..=257u64 {
        let bytes = intent(&fresh.host, fresh.b, seq, 1, n(2), IntentBody::EndTurn);
        let results = admit(&mut fresh.host, fresh.b, &bytes, 25 * seq);
        assert_eq!(
            results,
            vec![rejected(fresh.b, Some(seq), RejectionCode::FutureTick)],
            "M8 flood seq {seq} [{}]",
            hex(&bytes)
        );
        match seq {
            1 => first = bytes,
            2 => second = bytes,
            _ => {}
        }
    }
    assert_unchanged(flood_state, &fresh.host, "M8's 257 future-tick seqs");
    assert_bound(&fresh.host, fresh.b, "M8's 257 future-tick seqs");
    assert_eq!(
        fresh.host.cached_status_v3(fresh.b, 1),
        Ok(None),
        "M8 seq 257 evicted seq 1 after [{}]",
        hex(&first)
    );
    assert_eq!(
        fresh.host.cached_status_v3(fresh.b, 2),
        Ok(Some(ReceiptV3::Rejected(CodeV3::FutureTick))),
        "M8 seq 2 is still cached after [{}]",
        hex(&second)
    );
    let before = state(&fresh.host);
    let results = admit(&mut fresh.host, fresh.b, &first, 7_000);
    assert_eq!(
        results,
        vec![rejected(fresh.b, Some(1), RejectionCode::StaleSeq)],
        "M8 evicted replay [{}]",
        hex(&first)
    );
    assert_unchanged(before, &fresh.host, "M8 evicted replay");
    assert_bound(&fresh.host, fresh.b, "M8 evicted replay");

    // Control: a still-cached seq answers from the cache without executing.
    let results = admit(&mut fresh.host, fresh.b, &second, 7_000);
    assert_eq!(
        results,
        vec![rejected_cached(fresh.b, Some(2), RejectionCode::FutureTick)],
        "M8 cached replay [{}]",
        hex(&second)
    );
}

/// M9: seq 1 with different canonical bytes fences the session; the authority
/// does not move. Control: M7's identical bytes are cached and the session
/// stays bound.
#[test]
fn m9_conflicting_replay_fences() {
    let mut rig = new_rig();
    let frame = intent(
        &rig.host,
        rig.b,
        1,
        rig.host.server_tick(),
        n(2),
        IntentBody::EndTurn,
    );
    assert_eq!(
        admit(&mut rig.host, rig.b, &frame, 100),
        vec![applied(rig.b, 1)],
        "M9 first send [{}]",
        hex(&frame)
    );

    let conflict = intent(
        &rig.host,
        rig.b,
        1,
        rig.host.server_tick(),
        n(2),
        IntentBody::DeclareAction {
            ability: strike(),
            target: n(1),
        },
    );
    let before = state(&rig.host);
    let results = admit(&mut rig.host, rig.b, &conflict, 200);
    assert_eq!(
        results,
        vec![rejected(rig.b, Some(1), RejectionCode::SeqConflict)],
        "M9 conflict [{}]",
        hex(&conflict)
    );
    assert_unchanged(before, &rig.host, "M9 conflict");
    assert_eq!(
        rig.host.epoch(rig.b),
        Err(HostError::UnknownPeer),
        "M9 fenced the session after [{}]",
        hex(&conflict)
    );
}

/// M10: a non-minimal seq varint, the lowercase `k` ability alias and the
/// `O`/`0` alias all decode to the first frame and are cache hits.
#[test]
fn m10_non_canonical_retry_is_a_cache_hit() {
    let mut rig = new_rig();
    let epoch = rig.host.epoch(rig.b).expect("bound");
    let frame = IntentFrame {
        epoch,
        lane: LANE_COMBAT,
        seq: 1,
        observed_tick: rig.host.server_tick(),
        actor: n(2),
        body: IntentBody::DeclareAction {
            ability: strike(),
            target: n(1),
        },
    };
    let original = codec_v4::encode_intent(&frame).expect("encode");
    assert_eq!(
        admit(&mut rig.host, rig.b, &original, 100),
        vec![applied(rig.b, 1)],
        "M10 first send [{}]",
        hex(&original)
    );
    let executed = state(&rig.host);

    // (i) byte 18 is the seq; `0x81 0x00` is postcard's non-minimal `1`.
    assert_eq!(original[18], 0x01, "M10 seq byte");
    let mut nonminimal = original.clone();
    nonminimal.splice(18..19, [0x81u8, 0x00]);

    // (ii) the ability's canonical text, with its `K` in lowercase.
    let canonical = b"0000000000000000000000000K";
    let ability_at = find(&original, canonical).expect("M10 ability text");
    let mut lowercase = original.clone();
    lowercase[ability_at + canonical.len() - 1] = b'k';

    // (iii) the same, with the first `0` spelled `O`.
    let mut aliased = lowercase.clone();
    aliased[ability_at] = b'O';

    for (altered, now) in [(nonminimal, 200u64), (lowercase, 300), (aliased, 400)] {
        assert_ne!(altered, original, "M10 altered bytes differ");
        assert_eq!(
            codec_v4::decode_intent(&altered).expect("the decoder accepts the alias"),
            frame,
            "M10 altered bytes [{}] decode to the identical frame",
            hex(&altered)
        );
        let results = admit(&mut rig.host, rig.b, &altered, now);
        assert_eq!(
            results,
            vec![applied_cached(rig.b, 1)],
            "M10 retry at {now} [{}]",
            hex(&altered)
        );
        assert_unchanged(executed, &rig.host, "M10 retry");
        assert_bound(&rig.host, rig.b, "M10 retry");
    }
}
