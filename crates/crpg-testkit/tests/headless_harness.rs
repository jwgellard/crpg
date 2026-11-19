//! T041a acceptance suite (`tasks/T041.md`, H1–H8): the simulated network,
//! the arena over `HostLoop<LocalHost>`, the twin-session oracle, the scripts
//! and the independent mirror, all through `tests/support/headless.rs`.
//!
//! Expected values come from the contract, never from running the code. Each
//! case carries the control the contract names.

#[path = "support/headless.rs"]
mod headless;

use headless::*;

use crpg_core::EntityId;
use crpg_net::protocol_v3::ReceiptStatus as ReceiptV3;
use crpg_net::protocol_v4::IntentBody;
use crpg_sim::EncounterMode;

/// The started `combat_basic` combatants: hero first, goblin second.
fn combatants() -> (EntityId, EntityId) {
    let start = headless::start(EncounterMode::TurnBased, None);
    (start.hero, start.goblin)
}

/// H1: a clean reliable channel delivers immediately and in order; the
/// control's latency pushes every frame back by exactly two cycles.
#[test]
fn h1_clean_reliable_is_immediate_and_ordered() {
    let mut ch = Channel::new(ChannelKind::Reliable, 1, "h1");
    ch.send(5, vec![1], Faults::CLEAN);
    ch.send(5, vec![2], Faults::CLEAN);
    ch.send(5, vec![3], Faults::CLEAN);
    assert!(ch.take_due(4).is_empty());
    assert_eq!(ch.take_due(5), vec![vec![1], vec![2], vec![3]]);
    assert!(ch.take_due(5).is_empty());

    // Control: latency 2 -> nothing before now + 2, everything at now + 2.
    let mut ch2 = Channel::new(ChannelKind::Reliable, 1, "h1");
    let faults = Faults {
        latency: 2,
        ..Faults::CLEAN
    };
    ch2.send(5, vec![1], faults);
    ch2.send(5, vec![2], faults);
    ch2.send(5, vec![3], faults);
    assert!(ch2.take_due(6).is_empty());
    assert_eq!(ch2.take_due(7), vec![vec![1], vec![2], vec![3]]);
}

/// H2: a spec reliable channel never loses or doubles a frame, only time;
/// the control shows the clean channel delivers each frame at its own tick.
#[test]
fn h2_spec_reliable_loses_time_never_frames() {
    let mut ch = Channel::new(ChannelKind::Reliable, NET_SEED, "h2");
    for i in 0..2000u64 {
        ch.send(i, i.to_le_bytes().to_vec(), Faults::SPEC);
    }
    let mut arrivals: Vec<(u64, u64)> = Vec::new();
    for t in 0..=2100u64 {
        for bytes in ch.take_due(t) {
            let mut raw = [0u8; 8];
            raw.copy_from_slice(&bytes);
            arrivals.push((u64::from_le_bytes(raw), t));
        }
    }
    assert_eq!(arrivals.len(), 2000);
    for (k, &(i, t)) in arrivals.iter().enumerate() {
        assert_eq!(i, k as u64, "frame order");
        assert!(t >= i + 2, "frame {i} arrived early at {t}");
    }
    assert!(ch.stats().lost >= 1);
    assert!(arrivals.iter().any(|&(i, t)| t >= i + 6));

    // Control: CLEAN -> lost == 0 and every frame arrives at its own tick.
    let mut ch2 = Channel::new(ChannelKind::Reliable, NET_SEED, "h2");
    for i in 0..2000u64 {
        ch2.send(i, i.to_le_bytes().to_vec(), Faults::CLEAN);
    }
    for t in 0..=2100u64 {
        for bytes in ch2.take_due(t) {
            let mut raw = [0u8; 8];
            raw.copy_from_slice(&bytes);
            assert_eq!(u64::from_le_bytes(raw), t);
        }
    }
    assert_eq!(ch2.stats().lost, 0);
}

/// H3: a spec datagram channel loses, duplicates and reorders within the
/// jitter bound; the control shows the clean channel is lossless and ordered.
#[test]
fn h3_spec_datagrams_lose_duplicate_and_reorder() {
    let mut ch = Channel::new(ChannelKind::Datagram, NET_SEED, "h3");
    for i in 0..2000u64 {
        ch.send(i, i.to_le_bytes().to_vec(), Faults::SPEC);
    }
    let mut arrivals: Vec<(u64, u64)> = Vec::new();
    for t in 0..=2100u64 {
        for bytes in ch.take_due(t) {
            let mut raw = [0u8; 8];
            raw.copy_from_slice(&bytes);
            arrivals.push((u64::from_le_bytes(raw), t));
        }
    }
    let lost = ch.stats().lost;
    assert!(lost >= 1);
    let mut distinct: Vec<u64> = arrivals.iter().map(|&(i, _)| i).collect();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(distinct.len() as u64, 2000 - lost);
    let duplicated = ch.stats().duplicated;
    assert!(duplicated >= 1);
    assert_eq!(arrivals.len() as u64 - distinct.len() as u64, duplicated);
    assert!(
        arrivals.windows(2).any(|w| w[0].0 > w[1].0),
        "some later frame reorders before an earlier one"
    );
    for &(i, t) in &arrivals {
        assert!(
            t >= i + 2 && t <= i + 6,
            "frame {i} arrived out of bound at {t}"
        );
    }

    // Control: CLEAN -> each frame once, at its own tick, in order.
    let mut ch2 = Channel::new(ChannelKind::Datagram, NET_SEED, "h3");
    for i in 0..2000u64 {
        ch2.send(i, i.to_le_bytes().to_vec(), Faults::CLEAN);
    }
    for t in 0..=2100u64 {
        for bytes in ch2.take_due(t) {
            let mut raw = [0u8; 8];
            raw.copy_from_slice(&bytes);
            assert_eq!(u64::from_le_bytes(raw), t);
        }
    }
    assert_eq!(ch2.stats().lost, 0);
    assert_eq!(ch2.stats().duplicated, 0);
}

/// H4: a channel's draws depend only on seed, name and calls: same seed and
/// name repeat, a different name does not.
#[test]
fn h4_channels_are_deterministic() {
    fn run(seed: u64, name: &str) -> Vec<(u64, u64)> {
        let mut ch = Channel::new(ChannelKind::Reliable, seed, name);
        for i in 0..2000u64 {
            ch.send(i, i.to_le_bytes().to_vec(), Faults::SPEC);
        }
        let mut out = Vec::new();
        for t in 0..=2100u64 {
            for bytes in ch.take_due(t) {
                let mut raw = [0u8; 8];
                raw.copy_from_slice(&bytes);
                out.push((u64::from_le_bytes(raw), t));
            }
        }
        out
    }
    assert_eq!(run(NET_SEED, "h4"), run(NET_SEED, "h4"));
    assert_ne!(run(NET_SEED, "h4"), run(NET_SEED, "h4b"));
}

/// H5: `drop_next` discards exactly the next send, whatever its kind.
#[test]
fn h5_drop_next_discards_one() {
    let mut ch = Channel::new(ChannelKind::Reliable, 1, "h5");
    ch.drop_next();
    ch.send(0, vec![1], Faults::CLEAN);
    ch.send(0, vec![2], Faults::CLEAN);
    assert_eq!(ch.take_due(0), vec![vec![2]]);
    assert_eq!(ch.stats().discarded, 1);
    assert_eq!(ch.stats().sent, 1);
}

/// H6: the arena binds each client's twin right after it, delivers the
/// snapshot to every replica, and the twins and the two players converge.
#[test]
fn h6_arena_binds_twins_and_converges() {
    let (hero, goblin) = combatants();
    let seats = vec![
        Seat {
            name: "A",
            control: vec![hero],
            grants: Grants::Full,
            faults: Faults::CLEAN,
            script: Script::Idle,
            twin: true,
        },
        Seat {
            name: "B",
            control: vec![goblin],
            grants: Grants::Full,
            faults: Faults::CLEAN,
            script: Script::Idle,
            twin: true,
        },
    ];
    let mut arena = Arena::new(EncounterMode::TurnBased, None, seats, NET_SEED, true);

    // Peers are 1, 2, 3, 4 for A, A~, B, B~.
    assert_eq!(arena.clients.len(), 4);
    assert_eq!(arena.clients[0].name, "A");
    assert_eq!(arena.clients[0].peer.get(), 1);
    assert_eq!(arena.clients[0].twin_of, None);
    assert_eq!(arena.clients[1].name, "A~");
    assert_eq!(arena.clients[1].peer.get(), 2);
    assert_eq!(arena.clients[1].twin_of, Some(0));
    assert_eq!(arena.clients[2].name, "B");
    assert_eq!(arena.clients[2].peer.get(), 3);
    assert_eq!(arena.clients[2].twin_of, None);
    assert_eq!(arena.clients[3].name, "B~");
    assert_eq!(arena.clients[3].peer.get(), 4);
    assert_eq!(arena.clients[3].twin_of, Some(2));

    // Control: after run(0) nothing is delivered, so canon(A) has no rows.
    assert!(canon(&arena.client("A").replica).entities.is_empty());

    arena.run(30);
    for name in ["A", "A~", "B", "B~"] {
        let rep = &arena.client(name).replica;
        let ents = rep.v3().entities();
        assert_eq!(ents.len(), 2, "{name} rows");
        assert_eq!(ents[0].net_id, n(1));
        assert_eq!(ents[1].net_id, n(2));
        let positions = &canon(rep).positions;
        assert_eq!(
            positions.get(&n(1)),
            Some(&(pt(0, 0, 0), false)),
            "{name} n1 position"
        );
        assert_eq!(
            positions.get(&n(2)),
            Some(&(pt(0, 0, 0), false)),
            "{name} n2 position"
        );
    }
    let turns: Vec<_> = ["A", "A~", "B", "B~"]
        .iter()
        .map(|name| {
            let t = arena
                .client(name)
                .replica
                .v3()
                .turn()
                .cloned()
                .expect("a turn fact");
            (t.active, t.round)
        })
        .collect();
    assert_eq!(turns[0], turns[1]);
    assert_eq!(turns[0], turns[2]);
    assert_eq!(turns[0], turns[3]);
    assert_eq!(turns[0].0, Some(n(2)));
    assert_eq!(
        canon(&arena.client("A").replica),
        canon(&arena.client("A~").replica)
    );
    assert_eq!(
        canon(&arena.client("B").replica),
        canon(&arena.client("B~").replica)
    );
    assert_eq!(
        world_view(&arena.client("A").replica),
        world_view(&arena.client("B").replica)
    );
    assert_eq!(arena.authority(), arena.mirror_hash());
}

/// H7: a `Patrol` script moves once, waits for the receipt, then ends its
/// turn; the control shows the hero patrols to its own destination.
#[test]
fn h7_patrol_moves_then_ends_turn() {
    let (hero, goblin) = combatants();
    let seats = vec![
        Seat {
            name: "A",
            control: vec![hero],
            grants: Grants::Full,
            faults: Faults::CLEAN,
            script: Script::Idle,
            twin: true,
        },
        Seat {
            name: "B",
            control: vec![goblin],
            grants: Grants::Full,
            faults: Faults::CLEAN,
            script: Script::Patrol {
                actor: n(2),
                points: [pt(32768, 0, 0), pt(0, 0, 0)],
            },
            twin: true,
        },
    ];
    let mut arena = Arena::new(EncounterMode::TurnBased, None, seats, NET_SEED, true);
    arena.run(10);
    let b = arena.client("B");
    assert_eq!(b.sent.len(), 2, "B sends exactly a move then an end turn");
    assert_eq!(b.sent[0].seq, 1);
    assert_eq!(b.sent[0].actor, n(2));
    assert!(matches!(
        b.sent[0].body,
        IntentBody::DeclareMove { destination } if destination == pt(32768, 0, 0)
    ));
    assert_eq!(b.sent[1].seq, 2);
    assert_eq!(b.sent[1].actor, n(2));
    assert!(matches!(b.sent[1].body, IntentBody::EndTurn));
    assert_eq!(b.receipts.get(&1), Some(&vec![ReceiptV3::Applied]));
    assert_eq!(b.receipts.get(&2), Some(&vec![ReceiptV3::Applied]));
    assert_eq!(b.moves_applied(), 1);
    assert_eq!(
        b.replica.v3().turn().cloned().expect("a turn fact").active,
        Some(n(1)),
        "the turn is the hero's"
    );
    assert_eq!(arena.authority(), arena.mirror_hash());

    // Control: A with L1's Patrol too -> after run(20) A.sent starts with its
    // own move (seq 1) then its end turn (seq 2).
    let seats = vec![
        Seat {
            name: "A",
            control: vec![hero],
            grants: Grants::Full,
            faults: Faults::CLEAN,
            script: Script::Patrol {
                actor: n(1),
                points: [pt(32768, 0, 32768), pt(0, 0, 0)],
            },
            twin: true,
        },
        Seat {
            name: "B",
            control: vec![goblin],
            grants: Grants::Full,
            faults: Faults::CLEAN,
            script: Script::Patrol {
                actor: n(2),
                points: [pt(32768, 0, 0), pt(0, 0, 0)],
            },
            twin: true,
        },
    ];
    let mut arena = Arena::new(EncounterMode::TurnBased, None, seats, NET_SEED, true);
    arena.run(20);
    let a = arena.client("A");
    assert!(a.sent.len() >= 2);
    assert_eq!(a.sent[0].seq, 1);
    assert_eq!(a.sent[0].actor, n(1));
    assert!(matches!(
        a.sent[0].body,
        IntentBody::DeclareMove { destination } if destination == pt(32768, 0, 32768)
    ));
    assert_eq!(a.sent[1].seq, 2);
    assert_eq!(a.sent[1].actor, n(1));
    assert!(matches!(a.sent[1].body, IntentBody::EndTurn));
}

/// H8: settle empties every channel and pauses every client; resume restores
/// each client's fault profile.
#[test]
fn h8_settle_empties_and_resume_restores() {
    let (hero, goblin) = combatants();
    let seats = vec![
        Seat {
            name: "A",
            control: vec![hero],
            grants: Grants::Full,
            faults: Faults::SPEC,
            script: Script::Idle,
            twin: true,
        },
        Seat {
            name: "B",
            control: vec![goblin],
            grants: Grants::Full,
            faults: Faults::SPEC,
            script: Script::Idle,
            twin: true,
        },
    ];
    let mut arena = Arena::new(EncounterMode::TurnBased, None, seats, NET_SEED, true);
    arena.run(50);
    arena.settle();
    for client in &arena.clients {
        assert_eq!(client.in_flight(), 0);
        assert!(client.paused);
    }
    arena.resume();
    assert_eq!(arena.client("A").faults, Faults::SPEC);
    assert!(!arena.client("A").paused);
    assert_eq!(arena.client("A~").faults, Faults::CLEAN);
}
