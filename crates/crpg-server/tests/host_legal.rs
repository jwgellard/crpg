//! T047a contract tests: `legal_intents` and `replica_id` (two-keys suite).

use crpg_core::{EntityId, Ulid};
use crpg_net::protocol::{
    IntentBody, NetId, ReceiptStatus, RejectionCode, LANE_COMBAT, MAX_INTENT_FRAME_BYTES,
};
use crpg_server::host::{
    ControlGrant, DisclosureGrants, EntityDisclosure, Host, HostConfig, HostError,
    IngestDisposition, LegalIntent, PeerHandle, ProtocolSelection,
};
use crpg_sim::{legal_actions, CombatAction, HistoryWorld};

const FIXTURE: &str = include_str!("fixtures/three_combatants.json");
const V2: ProtocolSelection = ProtocolSelection::V2;

fn eid(index: u32) -> EntityId {
    serde_json::from_value(serde_json::json!({"index": index, "generation": 1})).expect("eid")
}
fn a() -> EntityId {
    eid(0)
}
fn b() -> EntityId {
    eid(1)
}
fn c() -> EntityId {
    eid(2)
}
fn strike() -> Ulid {
    Ulid::from_u128(603)
}
fn smite() -> Ulid {
    Ulid::from_u128(604)
}
fn whiff() -> Ulid {
    Ulid::from_u128(605)
}
fn encounter() -> Ulid {
    Ulid::from_u128(601)
}
fn n(raw: u64) -> NetId {
    NetId::new(raw).expect("nonzero")
}
fn fixture() -> HistoryWorld {
    let mut h: HistoryWorld = serde_json::from_str(FIXTURE).expect("fixture");
    h.acknowledge(h.last_sequence()).expect("ack");
    h
}
fn full(entity: EntityId) -> EntityDisclosure {
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
fn full_grants() -> DisclosureGrants {
    DisclosureGrants {
        entities: vec![full(a()), full(b()), full(c())],
        abilities: vec![strike(), smite(), whiff()],
        encounters: vec![encounter()],
        turn_round: true,
        encounter_end: true,
        turn_state: true,
    }
}
fn control(actors: &[EntityId]) -> ControlGrant {
    ControlGrant {
        actors: actors.to_vec(),
    }
}
fn all_actors() -> ControlGrant {
    control(&[a(), b(), c()])
}
fn config() -> HostConfig {
    HostConfig {
        protocol: V2,
        incarnation: 1,
    }
}
fn varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}
fn text(out: &mut Vec<u8>, v: &str) {
    varint(out, v.len() as u64);
    out.extend_from_slice(v.as_bytes());
}
fn declare(v: u8, e: [u8; 16], s: u64, o: u64, actor: u64, ab: Ulid, t: u64) -> Vec<u8> {
    let mut out = vec![v];
    out.extend_from_slice(&e);
    out.push(LANE_COMBAT);
    varint(&mut out, s);
    varint(&mut out, o);
    varint(&mut out, actor);
    out.push(0);
    text(&mut out, &ab.to_string());
    varint(&mut out, t);
    out
}
fn end_turn(v: u8, e: [u8; 16], s: u64, o: u64, actor: u64) -> Vec<u8> {
    let mut out = vec![v];
    out.extend_from_slice(&e);
    out.push(LANE_COMBAT);
    varint(&mut out, s);
    varint(&mut out, o);
    varint(&mut out, actor);
    out.push(1);
    out
}
fn li(actor: NetId, ability: Ulid, target: NetId) -> LegalIntent {
    LegalIntent {
        actor,
        body: IntentBody::DeclareAction { ability, target },
    }
}
fn end(actor: NetId) -> LegalIntent {
    LegalIntent {
        actor,
        body: IntentBody::EndTurn,
    }
}
// Hand-written literal: JV->b, JV->c, JW->b, JW->c, JX->b, JX->c, EndTurn (a=1,b=2,c=3).
fn expected_full() -> Vec<LegalIntent> {
    vec![
        li(n(1), strike(), n(2)),
        li(n(1), strike(), n(3)),
        li(n(1), smite(), n(2)),
        li(n(1), smite(), n(3)),
        li(n(1), whiff(), n(2)),
        li(n(1), whiff(), n(3)),
        end(n(1)),
    ]
}
fn expected_narrow() -> Vec<LegalIntent> {
    vec![
        li(n(1), strike(), n(2)),
        li(n(1), smite(), n(2)),
        li(n(1), whiff(), n(2)),
        end(n(1)),
    ]
}
fn expected_b_turn() -> Vec<LegalIntent> {
    vec![
        li(n(2), strike(), n(1)),
        li(n(2), strike(), n(3)),
        li(n(2), smite(), n(1)),
        li(n(2), smite(), n(3)),
        li(n(2), whiff(), n(1)),
        li(n(2), whiff(), n(3)),
        end(n(2)),
    ]
}
fn bind_full(now: u64) -> (Host, PeerHandle) {
    let mut h = Host::from_history(fixture(), config(), now).expect("wrap");
    let p = h.bind_peer(all_actors(), full_grants(), now).expect("bind");
    (h, p)
}
fn intent_bytes(host: &Host, peer: PeerHandle, seq: u64, o: &LegalIntent) -> Vec<u8> {
    let e = host.epoch(peer).expect("epoch");
    let t = host.server_tick();
    match &o.body {
        IntentBody::DeclareAction { ability, target } => {
            declare(2, e, seq, t, o.actor.get(), *ability, target.get())
        }
        IntentBody::EndTurn => end_turn(2, e, seq, t, o.actor.get()),
    }
}
fn staged(host: &mut Host, peer: PeerHandle, bytes: &[u8], now: u64) {
    assert_eq!(host.ingest(peer, bytes, now), Ok(IngestDisposition::Staged));
}
/// Totals, fixture rows included; never written to disk.
fn crowd(participants: usize, abilities: usize) -> HistoryWorld {
    let mut v: serde_json::Value = serde_json::from_str(FIXTURE).expect("fixture");
    let world = v.get_mut("world").expect("world");
    {
        let d = world
            .get_mut("combat")
            .expect("c")
            .get_mut("definition")
            .expect("d");
        d["allow_self_target"] = serde_json::Value::Bool(true);
        let list = d
            .get_mut("abilities")
            .expect("a")
            .as_array_mut()
            .expect("arr");
        for e in list.iter_mut() {
            e["allow_self_target"] = serde_json::Value::Bool(true);
        }
        let base = list[0].clone();
        for i in list.len()..abilities {
            let mut x = base.clone();
            x["ability"] =
                serde_json::Value::String(Ulid::from_u128(606 + (i - 3) as u128).to_string());
            list.push(x);
        }
    }
    let base = world.get("combatants").expect("cs")[0][1].clone();
    let have = world
        .get("combatants")
        .expect("cs")
        .as_array()
        .expect("arr")
        .len();
    for idx in have..participants {
        let key = 4 + (idx - have) as u64;
        let mut cb = base.clone();
        cb["placement"] =
            serde_json::Value::String(Ulid::from_u128(10_000 + idx as u128).to_string());
        cb["initiative"] = serde_json::Value::Number(serde_json::Number::from(key));
        let e = serde_json::json!({"index": idx, "generation": 1});
        world
            .get_mut("entities")
            .expect("e")
            .get_mut("slots")
            .expect("s")
            .as_array_mut()
            .expect("arr")
            .push(serde_json::json!({"generation": 1, "value": {}}));
        world
            .get_mut("combatants")
            .expect("cs")
            .as_array_mut()
            .expect("arr")
            .push(serde_json::Value::Array(vec![e.clone(), cb]));
        world
            .get_mut("timeline")
            .expect("t")
            .as_array_mut()
            .expect("arr")
            .push(serde_json::Value::Array(vec![
                serde_json::Value::Number(serde_json::Number::from(key)),
                e,
            ]));
    }
    let mut h: HistoryWorld = serde_json::from_value(v).expect("crowd loads");
    h.acknowledge(h.last_sequence()).expect("ack");
    h
}

#[test]
fn l1_closed_beats_unknown_peer() {
    let mut h = Host::from_history(fixture(), config(), 0).expect("wrap");
    let p1 = h.bind_peer(all_actors(), full_grants(), 0).expect("p1");
    let p2 = h.bind_peer(all_actors(), full_grants(), 0).expect("p2");
    assert!(h.legal_intents(p1).is_ok() && h.replica_id(p1, a()).is_ok());
    h.unbind_peer(p2);
    assert_eq!(h.legal_intents(p2), Err(HostError::UnknownPeer));
    assert_eq!(h.replica_id(p2, a()), Err(HostError::UnknownPeer));
    h.shutdown();
    assert_eq!(h.legal_intents(p1), Err(HostError::Closed));
    assert_eq!(h.replica_id(p1, a()), Err(HostError::Closed));
    assert_eq!(h.legal_intents(p2), Err(HostError::Closed));
    assert_eq!(h.replica_id(p2, a()), Err(HostError::Closed));
}

#[test]
fn l2_no_active_head_lists_nothing_and_maps_nothing() {
    let mut h = Host::new(7, config(), 0).expect("empty");
    let p = h.bind_peer(all_actors(), full_grants(), 0).expect("bind");
    assert_eq!(h.legal_intents(p), Ok(Vec::new()));
    assert_eq!(h.replica_id(p, a()), Ok(None));
    let (ch, cp) = bind_full(0); // control: fixture host is live
    assert!(!ch.legal_intents(cp).expect("list").is_empty());
    assert_eq!(ch.replica_id(cp, a()), Ok(Some(n(1))));
}

#[test]
fn l3_ineligible_active_head_lists_nothing() {
    let mut h = Host::from_history(fixture(), config(), 0).expect("wrap");
    let p = h
        .bind_peer(control(&[b()]), full_grants(), 0)
        .expect("bind");
    assert_eq!(h.legal_intents(p), Ok(Vec::new())); // active not controlled
    let mut h = Host::from_history(fixture(), config(), 0).expect("wrap");
    let g = DisclosureGrants {
        entities: vec![full(b()), full(c())],
        ..full_grants()
    };
    let p = h.bind_peer(all_actors(), g, 0).expect("bind");
    assert_eq!(h.legal_intents(p), Ok(Vec::new())); // active not mapped
    let mut h = Host::from_history(fixture(), config(), 0).expect("wrap");
    let mut g = full_grants();
    g.entities[0].action_actor = false;
    let p = h.bind_peer(all_actors(), g, 0).expect("bind");
    assert_eq!(h.legal_intents(p), Ok(Vec::new())); // present but not admissible
    let (ch, cp) = bind_full(0); // control restores the exact list
    assert_eq!(ch.legal_intents(cp).expect("list"), expected_full());
}

#[test]
fn l4_exact_list_order_and_oracle() {
    let (h, p) = bind_full(0);
    let got = h.legal_intents(p).expect("list");
    assert_eq!(got, expected_full());
    let oracle: Vec<CombatAction> = legal_actions(fixture().world(), a()).expect("oracle");
    let mapped: Vec<LegalIntent> = oracle
        .iter()
        .map(|x| match x {
            CombatAction::UseAbility {
                target, ability, ..
            } => li(n(1), *ability, if *target == b() { n(2) } else { n(3) }),
            CombatAction::EndTurn { .. } => end(n(1)),
            CombatAction::Stop { .. } => panic!("RT unreachable"),
        })
        .collect();
    assert_eq!(mapped, expected_full()); // literal matches the sim oracle
    assert_eq!(got, mapped);
    let mut narrowed = full_grants(); // one grant row drops three options
    narrowed.entities[2].action_target = false;
    let mut h = Host::from_history(fixture(), config(), 0).expect("wrap");
    let p = h.bind_peer(all_actors(), narrowed, 0).expect("bind");
    assert_eq!(h.legal_intents(p).expect("list"), expected_narrow());
    let (ch, cp) = bind_full(0); // control restores the dropped three
    assert_eq!(ch.legal_intents(cp).expect("list"), expected_full());
}

#[test]
fn l4b_too_many_options_through_public_host() {
    let grants = DisclosureGrants {
        entities: vec![full(a())],
        abilities: vec![strike(), smite(), whiff(), Ulid::from_u128(606)],
        encounters: vec![encounter()],
        turn_round: true,
        encounter_end: true,
        turn_state: true,
    };
    let mut probe = Host::from_history(crowd(1024, 4), config(), 0).expect("wrap");
    let p = probe
        .bind_peer(control(&[a()]), grants.clone(), 0)
        .expect("bind");
    let bh = probe.authority_hash();
    let be = probe.executions();
    let bl = probe.take_delivery_entries(p).expect("log");
    assert_eq!(
        probe.legal_intents(p),
        Err(HostError::Invariant("legal options exceed their bound"))
    );
    assert_eq!(probe.authority_hash(), bh); // whole query failed: nothing moved
    assert_eq!(probe.executions(), be);
    assert_eq!(probe.take_delivery_entries(p).expect("log"), bl);
    let mut narrow = grants; // control at exactly the bound, L5 keeps only EndTurn
    narrow.entities[0].action_target = false;
    narrow.abilities.push(Ulid::from_u128(607));
    let mut ch = Host::from_history(crowd(819, 5), config(), 0).expect("wrap");
    let cp = ch.bind_peer(control(&[a()]), narrow, 0).expect("bind");
    assert_eq!(ch.legal_intents(cp).expect("bound"), vec![end(n(1))]);
}

#[test]
fn l5a_full_grants_every_kept_option_is_applied() {
    for o in expected_full() {
        let (mut h, p) = bind_full(0);
        let bytes = intent_bytes(&h, p, 1, &o);
        staged(&mut h, p, &bytes, 0);
        let s = h.pump(0).expect("pump").results[0].status;
        assert_ne!(s, ReceiptStatus::Rejected(RejectionCode::NotAuthorized));
        assert_eq!(s, ReceiptStatus::Applied);
    }
}

#[test]
fn l5b_grant_refusal_drops_target_options() {
    let mut narrowed = full_grants();
    narrowed.entities[2].action_target = false; // c stays mapped
    let mut h = Host::from_history(fixture(), config(), 0).expect("wrap");
    let p = h
        .bind_peer(all_actors(), narrowed.clone(), 0)
        .expect("bind");
    assert_eq!(h.legal_intents(p).expect("list"), expected_narrow());
    for o in expected_narrow() {
        let mut f = Host::from_history(fixture(), config(), 0).expect("wrap");
        let fp = f
            .bind_peer(all_actors(), narrowed.clone(), 0)
            .expect("bind");
        let bytes = intent_bytes(&f, fp, 1, &o);
        staged(&mut f, fp, &bytes, 0);
        assert_eq!(
            f.pump(0).expect("pump").results[0].status,
            ReceiptStatus::Applied
        );
    }
    let mut f = Host::from_history(fixture(), config(), 0).expect("wrap");
    let fp = f.bind_peer(all_actors(), narrowed, 0).expect("bind");
    let denied = declare(
        2,
        f.epoch(fp).expect("e"),
        1,
        f.server_tick(),
        1,
        strike(),
        3,
    );
    staged(&mut f, fp, &denied, 0);
    assert_eq!(
        f.pump(0).expect("pump").results[0].status,
        ReceiptStatus::Rejected(RejectionCode::NotAuthorized)
    );
    let (ch, cp) = bind_full(0); // control: full host lists and applies the c option
    let wanted = li(n(1), strike(), n(3));
    assert!(ch.legal_intents(cp).expect("list").contains(&wanted));
    let (mut ch, cp) = bind_full(0);
    let bytes = intent_bytes(&ch, cp, 1, &wanted);
    staged(&mut ch, cp, &bytes, 0);
    assert_eq!(
        ch.pump(0).expect("pump").results[0].status,
        ReceiptStatus::Applied
    );
}

#[test]
fn l5c_unmapped_target_drops_options() {
    let g = DisclosureGrants {
        entities: vec![full(a()), full(b())],
        ..full_grants()
    };
    let mut h = Host::from_history(fixture(), config(), 0).expect("wrap");
    let p = h.bind_peer(all_actors(), g.clone(), 0).expect("bind");
    assert_eq!(h.legal_intents(p).expect("list"), expected_narrow()); // c has no NetId
    for o in expected_narrow() {
        let mut f = Host::from_history(fixture(), config(), 0).expect("wrap");
        let fp = f.bind_peer(all_actors(), g.clone(), 0).expect("bind");
        let bytes = intent_bytes(&f, fp, 1, &o);
        staged(&mut f, fp, &bytes, 0);
        assert_eq!(
            f.pump(0).expect("pump").results[0].status,
            ReceiptStatus::Applied
        );
    }
    let mut f = Host::from_history(fixture(), config(), 0).expect("wrap");
    let fp = f.bind_peer(all_actors(), g, 0).expect("bind");
    let denied = declare(
        2,
        f.epoch(fp).expect("e"),
        1,
        f.server_tick(),
        1,
        strike(),
        3,
    );
    staged(&mut f, fp, &denied, 0);
    assert_eq!(
        f.pump(0).expect("pump").results[0].status, // reverse.get(&3) is None
        ReceiptStatus::Rejected(RejectionCode::NotAuthorized)
    );
}

#[test]
fn l6a_reads_change_no_observable_state() {
    let (mut h, p) = bind_full(0);
    let first_bytes = end_turn(2, h.epoch(p).expect("e"), 1, h.server_tick(), 1);
    staged(&mut h, p, &first_bytes, 0);
    h.pump(0).expect("pump"); // active is now b
    let first_list = h.legal_intents(p).expect("list");
    assert_eq!(first_list, expected_b_turn());
    let first_id = h.replica_id(p, a()).expect("id");
    assert_eq!(first_id, Some(n(1)));
    let bh = h.authority_hash();
    let be = h.executions();
    let bt = h.server_tick();
    let bi = h.incarnation();
    let bp = h.protocol();
    let bc = h.is_closed();
    let bj = h.last_combat_error();
    let bl = h.last_capture();
    let ba = h.capture_acknowledged();
    let br = h.read_captures(0, 64).expect("r");
    let be2 = h.epoch(p).expect("e");
    let bs = h.cached_status(p, 1).expect("s");
    let bt2 = h.transfer_state(p).expect("t");
    let bd = h.take_delivery_entries(p).expect("d");
    for _ in 0..100 {
        assert_eq!(h.legal_intents(p).expect("list"), first_list);
        assert_eq!(h.replica_id(p, a()).expect("id"), first_id);
    }
    assert_eq!(h.authority_hash(), bh);
    assert_eq!(h.executions(), be);
    assert_eq!(h.server_tick(), bt);
    assert_eq!(h.incarnation(), bi);
    assert_eq!(h.protocol(), bp);
    assert_eq!(h.is_closed(), bc);
    assert_eq!(h.last_combat_error(), bj);
    assert_eq!(h.last_capture(), bl);
    assert_eq!(h.capture_acknowledged(), ba);
    assert_eq!(h.read_captures(0, 64).expect("r"), br);
    assert_eq!(h.epoch(p).expect("e"), be2);
    assert_eq!(h.cached_status(p, 1).expect("s"), bs);
    assert_eq!(h.transfer_state(p).expect("t"), bt2);
    assert_eq!(h.take_delivery_entries(p).expect("d"), bd);
    let second_bytes = end_turn(2, h.epoch(p).expect("e"), 2, h.server_tick(), 2);
    staged(&mut h, p, &second_bytes, 0);
    h.pump(0).expect("pump"); // control: a new valid intent for b moves all three
    assert_ne!(h.authority_hash(), bh);
    assert_ne!(h.executions(), be);
    assert_ne!(h.last_capture(), bl);
}

#[test]
fn l6b_intent_frame_bucket_untouched_by_reads() {
    for with_reads in [true, false] {
        let mut h = Host::from_history(fixture(), config(), 0).expect("wrap");
        let p = h.bind_peer(all_actors(), full_grants(), 0).expect("bind");
        for _ in 0..79 {
            staged(&mut h, p, &[2u8], 0);
        }
        if with_reads {
            for _ in 0..10 {
                h.legal_intents(p).expect("l");
                h.replica_id(p, a()).expect("r");
            }
        }
        staged(&mut h, p, &[2u8], 0); // 80th still staged
        assert_eq!(
            h.ingest(p, &[2u8], 0),
            Ok(IngestDisposition::Refused {
                status: ReceiptStatus::Rejected(RejectionCode::RateLimited)
            })
        );
    }
}

#[test]
fn l6c_intent_byte_bucket_untouched_by_reads() {
    for with_reads in [true, false] {
        let mut h = Host::from_history(fixture(), config(), 0).expect("wrap");
        let p = h.bind_peer(all_actors(), full_grants(), 0).expect("bind");
        let big = vec![2u8; 4096];
        assert!(big.len() <= MAX_INTENT_FRAME_BYTES);
        for _ in 0..31 {
            staged(&mut h, p, &big, 0);
        }
        if with_reads {
            for _ in 0..10 {
                h.legal_intents(p).expect("l");
                h.replica_id(p, a()).expect("r");
            }
        }
        staged(&mut h, p, &big, 0); // one byte spent by a read would refuse this 32nd
        assert_eq!(
            h.ingest(p, &[2u8], 0),
            Ok(IngestDisposition::Refused {
                // 33rd: frame room, no bytes
                status: ReceiptStatus::Rejected(RejectionCode::RateLimited)
            })
        );
    }
}

#[test]
fn l6d_failure_frame_bucket_untouched_by_reads() {
    for with_reads in [true, false] {
        let mut h = Host::from_history(fixture(), config(), 0).expect("wrap");
        let p = h.bind_peer(all_actors(), full_grants(), 0).expect("bind");
        h.take_delivery(p).expect("initial"); // 7-op full-state frame
        h.acknowledge_delivery(p, 1).expect("ack");
        let e = h.epoch(p).expect("e");
        let (mut ad, mut rj, mut dr) = (0u64, 0u64, 0u64);
        for seq in 1..=11u64 {
            if with_reads {
                h.legal_intents(p).expect("l");
                h.replica_id(p, a()).expect("r");
            }
            staged(&mut h, p, &end_turn(2, e, seq, 0, 99), 0); // actor 99: NotAuthorized
            let s = h.pump(0).expect("pump");
            ad += s.admitted as u64;
            rj += s.rejected as u64;
            dr += s.dropped_replies as u64;
        }
        assert_eq!((ad, rj, dr), (11, 11, 1));
        for seq in 1..=11u64 {
            assert_eq!(
                h.cached_status(p, seq).expect("s"),
                Some(ReceiptStatus::Rejected(RejectionCode::NotAuthorized))
            );
        }
        let entries = h.take_delivery_entries(p).expect("entries");
        assert_eq!(entries.len(), 10); // the burst of 10 keeps receipts 2..=11
        assert_eq!(
            entries.iter().map(|x| x.ordinal).collect::<Vec<u64>>(),
            (2..=11u64).collect::<Vec<u64>>()
        );
    }
}

#[test]
fn l6e_clock_and_handles() {
    let mut h = Host::from_history(fixture(), config(), 500).expect("wrap");
    let p = h.bind_peer(all_actors(), full_grants(), 500).expect("bind");
    h.legal_intents(p).expect("l");
    h.replica_id(p, a()).expect("r");
    let s = h.pump(500).expect("pump");
    assert_eq!((s.admitted, s.results.len()), (0, 0)); // empty summary
    assert_eq!(h.pump(499), Err(HostError::TimeRegression));
    assert_eq!(h.ingest(p, &[], 499), Err(HostError::TimeRegression));
    let q = h.bind_peer(all_actors(), full_grants(), 500).expect("q"); // only mover
    assert_eq!(q.get(), 2);
    let mut want = [0u8; 16];
    want[..8].copy_from_slice(&1u64.to_le_bytes());
    want[8..].copy_from_slice(&2u64.to_le_bytes());
    assert_eq!(h.epoch(q).expect("e"), want);
}

#[test]
fn l7_replica_id_mapping() {
    let mut h = Host::from_history(fixture(), config(), 0).expect("wrap");
    let p = h.bind_peer(all_actors(), full_grants(), 0).expect("bind");
    assert_eq!(h.replica_id(p, a()), Ok(Some(n(1)))); // plan_map mints in EntityId order
    assert_eq!(h.replica_id(p, b()), Ok(Some(n(2))));
    assert_eq!(h.replica_id(p, c()), Ok(Some(n(3))));
    assert_eq!(h.replica_id(p, eid(99)), Ok(None));
    let keep = h.replica_id(p, b()).expect("b"); // narrowing keeps 2 (compare, not literal)
    h.set_grants(
        p,
        DisclosureGrants {
            entities: vec![full(b()), full(c())],
            ..full_grants()
        },
    )
    .expect("n");
    assert_eq!(h.replica_id(p, a()), Ok(None));
    assert_eq!(h.replica_id(p, b()).expect("b"), keep);
    assert_eq!(h.replica_id(p, b()), Ok(Some(n(2))));
    let mut hide = full_grants(); // present:false then
    hide.entities[1].present = false;
    h.set_grants(p, hide).expect("hide");
    assert_eq!(h.replica_id(p, b()), Ok(None));
    let mut h = Host::from_history(fixture(), config(), 0).expect("wrap"); // re-widen mints fresh
    let p = h.bind_peer(all_actors(), full_grants(), 0).expect("bind");
    h.set_grants(
        p,
        DisclosureGrants {
            entities: vec![full(a()), full(b())],
            ..full_grants()
        },
    )
    .expect("n");
    assert_eq!(h.replica_id(p, c()), Ok(None));
    h.set_grants(p, full_grants()).expect("w");
    let renewed = h.replica_id(p, c()).expect("c");
    assert!(renewed.is_some() && renewed != Some(n(3))); // never reuses 3
    h.unbind_peer(p); // control: L1 UnknownPeer
    assert_eq!(h.replica_id(p, a()), Err(HostError::UnknownPeer));
}
