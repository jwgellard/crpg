//! T125 audit gaps (tests only).

use crpg_core::{EntityId, Ulid};
use crpg_net::protocol::{ReceiptStatus, RejectionCode, LANE_COMBAT};
use crpg_server::checkpoint::load_checkpoint;
use crpg_server::host::{
    ControlGrant, DisclosureGrants, EntityDisclosure, Host, HostConfig, IngestDisposition,
    ProtocolSelection,
};
use crpg_sim::HistoryWorld;
use serde_json::json;

const FIXTURE: &str = include_str!("fixtures/three_combatants.json");

const V2: ProtocolSelection = ProtocolSelection::V2;

fn eid(index: u32) -> EntityId {
    serde_json::from_value(json!({"index": index, "generation": 1})).expect("valid entity id")
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
        abilities: vec![strike(), smite()],
        encounters: vec![Ulid::from_u128(601)],
        turn_round: true,
        encounter_end: true,
        turn_state: true,
    }
}
fn all_actors() -> ControlGrant {
    ControlGrant {
        actors: vec![a(), b(), c()],
    }
}

fn fixture() -> HistoryWorld {
    let mut history: HistoryWorld = serde_json::from_str(FIXTURE).expect("fixture");
    history.acknowledge(history.last_sequence()).expect("ack");
    history
}

fn varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push((value as u8) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}
fn text(out: &mut Vec<u8>, value: &str) {
    varint(out, value.len() as u64);
    out.extend_from_slice(value.as_bytes());
}
fn declare(
    version: u8,
    epoch: [u8; 16],
    seq: u64,
    observed: u64,
    actor: u64,
    ability: Ulid,
    target: u64,
) -> Vec<u8> {
    let mut out = vec![version];
    out.extend_from_slice(&epoch);
    out.push(LANE_COMBAT);
    varint(&mut out, seq);
    varint(&mut out, observed);
    varint(&mut out, actor);
    out.push(0);
    text(&mut out, &ability.to_string());
    varint(&mut out, target);
    out
}
fn end_turn(version: u8, epoch: [u8; 16], seq: u64, observed: u64, actor: u64) -> Vec<u8> {
    let mut out = vec![version];
    out.extend_from_slice(&epoch);
    out.push(LANE_COMBAT);
    varint(&mut out, seq);
    varint(&mut out, observed);
    varint(&mut out, actor);
    out.push(1);
    out
}

// #63
#[test]
fn checkpoint_rejects_capture_history_gap() {
    let config = HostConfig {
        protocol: V2,
        incarnation: 1,
    };
    let mut host = Host::from_history(fixture(), config, 0).expect("wrap");
    let peer = host
        .bind_peer(all_actors(), full_grants(), 0)
        .expect("bind");
    let epoch = host.epoch(peer).expect("epoch");
    let bytes = declare(2, epoch, 1, 0, 1, strike(), 2);
    host.ingest(peer, &bytes, 0).expect("ingest");
    host.pump(0).expect("pump");
    let bytes = end_turn(2, epoch, 2, 0, 2);
    host.ingest(peer, &bytes, 0).expect("ingest");
    host.pump(0).expect("pump");
    let saved = host.save_checkpoint().expect("save");
    let text = String::from_utf8(saved).unwrap();
    let start = text.find("\"captured\":[").expect("captured");
    let end = text.rfind("],\"capture_acknowledged\":0}").expect("tail");
    let second = text.find("\"capture_seq\":2").expect("second");
    let obj_start = text[..second].rfind('{').expect("obj");
    let gapped = format!(
        "{}{}{}",
        &text[..start + "\"captured\":[".len()],
        &text[obj_start..end],
        &text[end..]
    );
    let res = load_checkpoint(gapped.as_bytes(), 2, 0);
    assert!(res.is_err());
}

// #67
#[test]
fn ingress_caps_refuse_and_admit_after_drain() {
    let config = HostConfig {
        protocol: V2,
        incarnation: 1,
    };
    let mut host = Host::new(1, config, 0).expect("new");
    let peer = host
        .bind_peer(all_actors(), full_grants(), 0)
        .expect("binds");
    let epoch = host.epoch(peer).expect("epoch");
    // Fill per-peer ingress cap (128 frames).
    let mut now = 0u64;
    for i in 0..128u64 {
        now += 100;
        let bytes = end_turn(2, epoch, i + 1, 0, 1);
        let disp = host.ingest(peer, &bytes, now).expect("ingest");
        assert_eq!(disp, IngestDisposition::Staged, "frame {}", i);
    }
    // Next ingest refused as QueueFull.
    now += 100;
    let bytes = end_turn(2, epoch, 129, 0, 1);
    let disp = host.ingest(peer, &bytes, now).expect("ingest");
    assert_eq!(
        disp,
        IngestDisposition::Refused {
            status: ReceiptStatus::Rejected(RejectionCode::QueueFull)
        }
    );
    // Drain (pump) frees ingress.
    let _ = host.pump(now).expect("pump");
    // Admits again.
    now += 100;
    let bytes = end_turn(2, epoch, 129, 0, 1);
    let disp = host.ingest(peer, &bytes, now).expect("ingest");
    assert_eq!(disp, IngestDisposition::Staged);
}
