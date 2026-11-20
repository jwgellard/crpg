//! T041a acceptance suite (`tasks/T041.md`, H1–H8): the simulated network,
//! the arena over `HostLoop<LocalHost>`, the twin-session oracle, the scripts
//! and the independent mirror, all through `tests/support/headless.rs`.
//!
//! Expected values come from the contract, never from running the code. Each
//! case carries the control the contract names.

#[path = "support/headless.rs"]
mod headless;

use headless::*;

use crpg_core::{EntityId, Ulid};
use crpg_net::facts_v3::EncounterGrants;
use crpg_net::movement_v4::{self, MoveBody, MoveDown, MoveSample, SampleEntry};
use crpg_net::protocol_v3::{ReceiptStatus as ReceiptV3, RejectionCode};
use crpg_net::protocol_v4::{IntentBody, Point};
use crpg_server::host::{
    DisclosureGrants, EntityDisclosure, EntityFactGrant, FactGrants, DEFAULT_HOST_GRANTS,
};
use crpg_sim::{history_hash, EncounterClock, EncounterMode};

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

    // B-1: a reliable channel is ordered: a later send never overtakes, so
    // the earlier frame keeps its (now clamped) earlier order at the later due.
    let mut ch3 = Channel::new(ChannelKind::Reliable, 1, "h1-due");
    ch3.send(
        0,
        vec![1],
        Faults {
            latency: 3,
            ..Faults::CLEAN
        },
    );
    ch3.send(0, vec![2], Faults::CLEAN);
    assert_eq!(ch3.take_due(2), Vec::<Vec<u8>>::new(), "nothing due yet");
    assert_eq!(ch3.take_due(3), vec![vec![1], vec![2]]);
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

    // B-2: `(due, order)` again, for a datagram: send order and due order
    // differ, so the later send is the earlier arrival.
    let mut ch3 = Channel::new(ChannelKind::Datagram, NET_SEED, "h3-due");
    ch3.send(
        0,
        vec![1],
        Faults {
            latency: 3,
            ..Faults::CLEAN
        },
    );
    ch3.send(0, vec![2], Faults::CLEAN);
    assert_eq!(ch3.take_due(3), vec![vec![2], vec![1]]);
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
    // The seed is a draw input too: two seeds, one name, different schedules.
    assert_ne!(run(NET_SEED, "h4"), run(NET_SEED + 1, "h4"));
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

    // B-1: `drop_next` makes no draw, so a channel that discarded one send has
    // drawn exactly what a channel that never discarded draws; both deliver
    // the same frames at the same cycles under the same profile.
    let faults = Faults {
        loss_percent: 25,
        ..Faults::SPEC
    };
    let mut dropped = Channel::new(ChannelKind::Reliable, NET_SEED, "h5-draw");
    dropped.drop_next();
    dropped.send(0, vec![9], faults);
    let mut control = Channel::new(ChannelKind::Reliable, NET_SEED, "h5-draw");
    for i in 0..500u64 {
        let frame = i.to_le_bytes().to_vec();
        dropped.send(i, frame.clone(), faults);
        control.send(i, frame, faults);
    }
    assert_eq!(dropped.stats().discarded, 1);
    assert_eq!(dropped.stats().sent, 500);
    assert_eq!(control.stats().sent, 500);
    assert!(
        control.stats().lost >= 1,
        "the profile exercises the draw stream"
    );
    assert_eq!(
        drain_channel(&mut dropped, 900),
        drain_channel(&mut control, 900)
    );
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

    // B-14: `canon` carries each replica's rows and turn, and `twin` names the
    // twin rather than the client itself.
    let canon_a = canon(&arena.client("A").replica);
    assert_eq!(canon_a.entities.len(), 2, "canon keeps both rows");
    assert_eq!(canon_a.entities[0].net_id, n(1));
    assert_eq!(canon_a.entities[1].net_id, n(2));
    assert_eq!(
        canon_a.turn,
        arena.client("A").replica.v3().turn().cloned(),
        "canon keeps the turn fact"
    );
    assert!(canon_a.turn.is_some(), "the turn fact is disclosed");
    assert_eq!(arena.twin("A").name, "A~");
    assert_eq!(arena.twin("A").peer.get(), 2);
    assert_eq!(arena.twin("B").peer.get(), 4);
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
    // B-13: every frame's `observed_tick` is the replica's server tick at the
    // moment the script ran (the state after the previous cycle).
    for _ in 0..10 {
        let before = arena.client("B").replica.v3().server_tick();
        let sent = arena.client("B").sent.len();
        arena.step();
        for frame in &arena.client("B").sent[sent..] {
            assert_eq!(
                frame.observed_tick, before,
                "seq {} observed tick",
                frame.seq
            );
        }
    }
    let b = arena.client("B");
    assert!(
        b.sent.iter().any(|f| f.observed_tick > 0),
        "a late frame carries a nonzero observed tick"
    );
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
    assert_eq!(
        canon(&b.replica).positions.get(&n(2)),
        Some(&(pt(32768, 0, 0), false)),
        "the arrival sample moved B's replica"
    );
    assert_eq!(
        canon(&b.replica).timeline,
        b.replica.v3().timeline().map(<[_]>::to_vec),
        "canon keeps the timeline image"
    );
    assert!(
        canon(&b.replica).timeline.is_some(),
        "an action exposes the timeline"
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
    let before = arena.cycle;
    arena.settle();
    assert_eq!(
        arena.cycle,
        before + SETTLE_CYCLES,
        "a settle runs SETTLE_CYCLES cycles"
    );
    for client in &arena.clients {
        assert_eq!(client.in_flight(), 0);
        assert!(client.paused);
        assert_eq!(
            client.faults,
            Faults::CLEAN,
            "{} settles under a clean profile",
            client.name
        );
    }
    arena.resume();
    assert_eq!(arena.client("A").faults, Faults::SPEC);
    assert!(!arena.client("A").paused);
    assert_eq!(arena.client("A~").faults, Faults::CLEAN);

    // B-7: a settle saves the profile it finds, not the one the seat was
    // built with.
    arena.client_mut("A").faults = Faults::CLEAN;
    arena.settle();
    arena.resume();
    assert_eq!(
        arena.client("A").faults,
        Faults::CLEAN,
        "the second settle saved the profile it found"
    );
}

// ---------------------------------------------------------------------------
// Gaps added by the T041a fix round: the folded-away branches of B-1–B-6.
// ---------------------------------------------------------------------------

/// Drains every frame `ch` will deliver up to `until`, tagged with its cycle.
fn drain_channel(ch: &mut Channel, until: u64) -> Vec<(u64, Vec<u8>)> {
    let mut out = Vec::new();
    for t in 0..=until {
        for bytes in ch.take_due(t) {
            out.push((t, bytes));
        }
    }
    out
}

/// B-1: the loss loop stops at exactly `MAX_LOSSES` (8), each loss costs `rto`
/// (4 cycles), and the frame still gets through.
#[test]
fn reliable_loss_cap_is_max_losses_and_rto() {
    let faults = Faults {
        loss_percent: 100,
        rto: 4,
        ..Faults::CLEAN
    };
    let mut ch = Channel::new(ChannelKind::Reliable, 7, "loss-cap");
    ch.send(0, vec![1], faults);
    let due = u64::from(MAX_LOSSES) * 4;
    assert!(
        ch.take_due(due - 1).is_empty(),
        "no delivery before MAX_LOSSES x rto"
    );
    assert_eq!(ch.take_due(due), vec![vec![1]]);
    assert_eq!(ch.stats().lost, u64::from(MAX_LOSSES));
    assert_eq!(ch.stats().sent, 1);
}

/// B-1: a reliable channel draws its jitter every send: with no loss and no
/// latency, frame `i` arrives in `[i, i + 4]`, never early, and some frame is
/// delayed by a nonzero draw.
#[test]
fn reliable_jitter_delays_some_frames() {
    let faults = Faults {
        jitter: 4,
        ..Faults::CLEAN
    };
    let mut ch = Channel::new(ChannelKind::Reliable, NET_SEED, "h-jitter");
    for i in 0..2000u64 {
        ch.send(i, i.to_le_bytes().to_vec(), faults);
    }
    let mut count = 0u64;
    let mut last = 0u64;
    let mut late = 0u64;
    for t in 0..=2100u64 {
        for bytes in ch.take_due(t) {
            let mut raw = [0u8; 8];
            raw.copy_from_slice(&bytes);
            let i = u64::from_le_bytes(raw);
            assert_eq!(i, count, "a reliable channel keeps send order");
            assert!(t >= i && t <= i + 4, "frame {i} arrived at {t}");
            assert!(t >= last, "delivery never goes back");
            last = t;
            if t > i {
                late += 1;
            }
            count += 1;
        }
    }
    assert_eq!(count, 2000);
    assert_eq!(ch.stats().lost, 0);
    assert!(late >= 1, "some nonzero jitter draw delays its frame");
}

/// B-2: a duplicated datagram's copy draws its own jitter, so some frame has
/// both copies strictly later than its send cycle.
#[test]
fn datagram_duplicate_copy_is_jittered() {
    let faults = Faults {
        dup_percent: 100,
        jitter: 4,
        ..Faults::CLEAN
    };
    let mut ch = Channel::new(ChannelKind::Datagram, NET_SEED, "h-dup-jitter");
    for i in 0..200u64 {
        ch.send(i, i.to_le_bytes().to_vec(), faults);
    }
    let mut arrivals: std::collections::BTreeMap<u64, Vec<u64>> = std::collections::BTreeMap::new();
    for t in 0..=300u64 {
        for bytes in ch.take_due(t) {
            let mut raw = [0u8; 8];
            raw.copy_from_slice(&bytes);
            arrivals.entry(u64::from_le_bytes(raw)).or_default().push(t);
        }
    }
    assert_eq!(arrivals.len(), 200);
    assert_eq!(ch.stats().duplicated, 200);
    let mut both_late = 0;
    for (&i, times) in &arrivals {
        assert_eq!(times.len(), 2, "frame {i} is copied exactly once");
        for &t in times {
            assert!(t >= i && t <= i + 4, "frame {i} arrived at {t}");
        }
        if times.iter().all(|&t| t > i) {
            both_late += 1;
        }
    }
    assert!(
        both_late >= 1,
        "both copies of some frame carry a nonzero jitter draw"
    );
}

/// B-3/B-4: `start` honours its mode and `round_ticks`, and the two grant
/// sets are exactly the contract's Full and Observer sets.
#[test]
fn start_honours_mode_and_round_ticks_and_the_grants_are_exact() {
    let started = headless::start(EncounterMode::RealTime, Some(4));
    let combat = started.mirror.world().combat().expect("an encounter");
    assert_eq!(combat.mode, EncounterMode::RealTime);
    assert_eq!(
        combat.clock,
        Some(EncounterClock {
            round_ticks: 4,
            elapsed: 0
        })
    );
    assert_eq!(combat.active, None, "a real-time start schedules no turn");
    // B-3: the hero's placement is 0E and the goblin's 0F.
    let hero_placement: Ulid = "0000000000000000000000000E".parse().expect("a ULID");
    let goblin_placement: Ulid = "0000000000000000000000000F".parse().expect("a ULID");
    let combatants = started.mirror.world().combatants();
    assert_eq!(
        combatants.get(started.hero).expect("the hero").placement(),
        hero_placement
    );
    assert_eq!(
        combatants
            .get(started.goblin)
            .expect("the goblin")
            .placement(),
        goblin_placement
    );
    assert_eq!(started.host.authority_hash(), history_hash(&started.mirror));

    let full_entity = |entity| EntityDisclosure {
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
    };
    let observer_entity = |entity| EntityDisclosure {
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
    };
    let (hero, goblin) = (started.hero, started.goblin);
    assert_eq!(
        disclosure(Grants::Full, &started),
        DisclosureGrants {
            entities: vec![full_entity(hero), full_entity(goblin)],
            abilities: vec![strike()],
            encounters: vec![first_blood()],
            turn_round: true,
            encounter_end: true,
            turn_state: true,
        }
    );
    assert_eq!(
        disclosure(Grants::Observer, &started),
        DisclosureGrants {
            entities: vec![observer_entity(hero), observer_entity(goblin)],
            abilities: Vec::new(),
            encounters: Vec::new(),
            turn_round: false,
            encounter_end: false,
            turn_state: true,
        }
    );
    let all_facts = |entity| EntityFactGrant {
        entity,
        pools: true,
        effects: true,
        stats: true,
        tags: true,
    };
    assert_eq!(
        fact_grants(Grants::Full, &started),
        FactGrants {
            entities: vec![all_facts(hero), all_facts(goblin)],
            encounter: EncounterGrants {
                mode: true,
                pending_mode: true,
                clock: true,
                timeline: true,
            },
            host: DEFAULT_HOST_GRANTS,
        }
    );
    assert_eq!(
        fact_grants(Grants::Observer, &started),
        FactGrants::default()
    );
}

/// B-5: each client's four channels are built with the seat's name, the run's
/// `net_seed` and the channel's kind.
#[test]
fn arena_channels_use_their_seed_name_and_kind() {
    let (hero, _goblin) = combatants();
    let seats = vec![Seat {
        name: "A",
        control: vec![hero],
        grants: Grants::Full,
        faults: Faults::CLEAN,
        script: Script::Idle,
        twin: false,
    }];
    let mut arena = Arena::new(EncounterMode::TurnBased, None, seats, NET_SEED, false);
    let faults = Faults {
        loss_percent: 40,
        dup_percent: 0,
        latency: 1,
        jitter: 4,
        rto: 3,
    };
    let frame = |i: u64| vec![i as u8, (i >> 8) as u8];

    let mut expected_up = Channel::new(ChannelKind::Reliable, NET_SEED, "A/up");
    for i in 0..64u64 {
        arena
            .client_mut("A")
            .channel_mut(Which::Up)
            .send(i, frame(i), faults);
        expected_up.send(i, frame(i), faults);
    }
    assert!(
        expected_up.stats().lost >= 1,
        "the profile exercises the draws"
    );
    assert_eq!(
        drain_channel(arena.client_mut("A").channel_mut(Which::Up), 500),
        drain_channel(&mut expected_up, 500),
        "the up channel is seeded with net_seed on stream \"A/up\""
    );

    let mut expected_down0 = Channel::new(ChannelKind::Reliable, NET_SEED, "A/down0");
    for i in 0..64u64 {
        arena
            .client_mut("A")
            .channel_mut(Which::Down0)
            .send(i, frame(i), faults);
        expected_down0.send(i, frame(i), faults);
    }
    assert_eq!(
        drain_channel(arena.client_mut("A").channel_mut(Which::Down0), 500),
        drain_channel(&mut expected_down0, 500),
        "the down0 channel draws on stream \"A/down0\""
    );

    let mut expected_down1 = Channel::new(ChannelKind::Reliable, NET_SEED, "A/down1");
    for i in 0..64u64 {
        arena
            .client_mut("A")
            .channel_mut(Which::Down1)
            .send(i, frame(i), faults);
        expected_down1.send(i, frame(i), faults);
    }
    assert_eq!(
        drain_channel(arena.client_mut("A").channel_mut(Which::Down1), 500),
        drain_channel(&mut expected_down1, 500),
        "the down1 channel draws on stream \"A/down1\""
    );

    let lossy = Faults {
        loss_percent: 100,
        ..Faults::CLEAN
    };
    arena
        .client_mut("A")
        .channel_mut(Which::Datagram)
        .send(0, vec![1], lossy);
    assert_eq!(
        arena
            .client_mut("A")
            .channel_mut(Which::Datagram)
            .stats()
            .lost,
        1,
        "a datagram drops the frame instead of retransmitting it"
    );
    assert!(drain_channel(arena.client_mut("A").channel_mut(Which::Datagram), 100).is_empty());
}

/// B-6.2/B-6.6: `step` returns its cycle's admitted lane-0 intents, each with
/// the client's own index, and records every receipt's cycle.
#[test]
fn step_returns_its_admitted_intents() {
    let (hero, goblin) = combatants();
    let seats = vec![
        Seat {
            name: "A",
            control: vec![hero],
            grants: Grants::Full,
            faults: Faults::CLEAN,
            script: Script::Idle,
            twin: false,
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
            twin: false,
        },
    ];
    let mut arena = Arena::new(EncounterMode::TurnBased, None, seats, NET_SEED, true);
    let peer_b = arena.client("B").peer;
    let mut batches: Vec<(u64, Vec<Admitted>)> = Vec::new();
    for cycle in 0..10u64 {
        let admitted = arena.step();
        assert_eq!(
            arena.authority(),
            arena.mirror_hash(),
            "the mirror after cycle {cycle}"
        );
        if !admitted.is_empty() {
            batches.push((cycle, admitted));
        }
    }
    assert_eq!(batches.len(), 2, "B's move and its end turn, one per cycle");
    let (c1, first) = &batches[0];
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].client, 1, "B is the second client in the vector");
    assert_eq!(first[0].peer, peer_b);
    assert_eq!(first[0].frame, arena.client("B").sent[0]);
    let (c2, second) = &batches[1];
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].client, 1);
    assert_eq!(second[0].peer, peer_b);
    assert_eq!(second[0].frame.seq, 2);
    assert!(matches!(second[0].frame.body, IntentBody::EndTurn));
    assert_eq!(
        arena.client("B").receipt_cycles,
        vec![(1, *c1), (2, *c2)],
        "each receipt records the cycle it was delivered in"
    );
}

/// B-6.4: a seat running `Intruder` is never applied to the mirror, so the
/// authority stays equal to it while the host refuses every intrusion.
#[test]
fn intruder_frames_are_not_mirrored() {
    let (hero, _goblin) = combatants();
    let seats = vec![
        Seat {
            name: "A",
            control: vec![hero],
            grants: Grants::Full,
            faults: Faults::CLEAN,
            script: Script::Idle,
            twin: false,
        },
        Seat {
            name: "O",
            control: Vec::new(),
            grants: Grants::Observer,
            faults: Faults::CLEAN,
            script: Script::Intruder {
                actor: n(1),
                every: 4,
            },
            twin: false,
        },
    ];
    let mut arena = Arena::new(EncounterMode::TurnBased, None, seats, NET_SEED, true);
    arena.run(20);
    let o = arena.client("O");
    assert_eq!(o.sent.len(), 4, "cycles 4, 8, 12 and 16");
    assert!(
        o.sent.iter().all(|f| f.actor == n(1)),
        "the intrusion targets the hero"
    );
    assert!(
        o.sent.iter().all(|f| matches!(f.body, IntentBody::EndTurn)),
        "an intrusion is an EndTurn, not a move"
    );
    assert_eq!(o.receipts.len(), 4);
    for seq in 1..=4u64 {
        assert_eq!(
            o.receipts.get(&seq),
            Some(&vec![ReceiptV3::Rejected(RejectionCode::NotAuthorized)]),
            "intrusion {seq}"
        );
    }
    assert_eq!(
        arena.authority(),
        arena.mirror_hash(),
        "the intruder's frames never touch the mirror"
    );
}

/// B-6.1/B-7: `settle` pauses every script, so an `Intruder` sends nothing
/// while it runs; `resume` unpauses them again.
#[test]
fn settle_pauses_every_script() {
    let (hero, goblin) = combatants();
    let seats = vec![
        Seat {
            name: "A",
            control: vec![hero],
            grants: Grants::Full,
            faults: Faults::SPEC,
            script: Script::Idle,
            twin: false,
        },
        Seat {
            name: "B",
            control: vec![goblin],
            grants: Grants::Full,
            faults: Faults::SPEC,
            script: Script::Idle,
            twin: false,
        },
        Seat {
            name: "O",
            control: Vec::new(),
            grants: Grants::Observer,
            faults: Faults::SPEC,
            script: Script::Intruder {
                actor: n(1),
                every: 2,
            },
            twin: false,
        },
    ];
    let mut arena = Arena::new(EncounterMode::TurnBased, None, seats, NET_SEED, false);
    arena.run(10);
    assert_eq!(arena.client("O").sent.len(), 4, "cycles 2, 4, 6 and 8");
    arena.settle();
    assert_eq!(arena.client("O").sent.len(), 4, "a settle runs no script");
    for client in &arena.clients {
        assert!(client.paused, "{} is paused after a settle", client.name);
    }
    arena.resume();
    for client in &arena.clients {
        assert!(!client.paused, "{} is unpaused after a resume", client.name);
    }
}

/// B-8: a `Patrol` sends nothing further until the receipt for the intent it
/// is awaiting arrives; under latency the wait is several cycles long.
#[test]
fn patrol_waits_for_the_receipt() {
    let (hero, goblin) = combatants();
    let seats = vec![
        Seat {
            name: "A",
            control: vec![hero],
            grants: Grants::Full,
            faults: Faults::CLEAN,
            script: Script::Idle,
            twin: false,
        },
        Seat {
            name: "B",
            control: vec![goblin],
            grants: Grants::Full,
            faults: Faults {
                latency: 3,
                ..Faults::CLEAN
            },
            script: Script::Patrol {
                actor: n(2),
                points: [pt(32768, 0, 0), pt(0, 0, 0)],
            },
            twin: false,
        },
    ];
    let mut arena = Arena::new(EncounterMode::TurnBased, None, seats, NET_SEED, false);
    for cycle in 0..60u64 {
        arena.step();
        let b = arena.client("B");
        if b.receipts.contains_key(&1) && b.receipts.contains_key(&2) {
            break;
        }
        if !b.sent.is_empty() && !b.receipts.contains_key(&1) {
            assert_eq!(
                b.sent.len(),
                1,
                "cycle {cycle}: B sent before its receipt arrived"
            );
        }
    }
    let b = arena.client("B");
    assert_eq!(b.sent.len(), 2, "one move then one end turn");
    assert!(matches!(b.sent[0].body, IntentBody::DeclareMove { .. }));
    assert!(matches!(b.sent[1].body, IntentBody::EndTurn));
    assert_eq!(b.receipts.get(&1), Some(&vec![ReceiptV3::Applied]));
    assert_eq!(b.receipts.get(&2), Some(&vec![ReceiptV3::Applied]));
    assert_eq!(b.moves_applied(), 1);
}

/// B-6.6: a `DeclareMove` refused by the host does not count as applied.
#[test]
fn a_refused_declare_move_is_not_counted() {
    let (hero, goblin) = combatants();
    let seats = vec![
        Seat {
            name: "A",
            control: vec![hero],
            grants: Grants::Full,
            faults: Faults::CLEAN,
            script: Script::Idle,
            twin: false,
        },
        Seat {
            name: "B",
            control: vec![goblin],
            grants: Grants::Full,
            faults: Faults::CLEAN,
            // The opposite corner is sqrt(2) m against a one-step pool, so the
            // move is refused.
            script: Script::Fight {
                actor: n(2),
                target: n(1),
                first_move: pt(65536, 0, 65536),
            },
            twin: false,
        },
    ];
    let mut arena = Arena::new(EncounterMode::TurnBased, None, seats, NET_SEED, false);
    arena.run(10);
    let b = arena.client("B");
    assert_eq!(b.sent.len(), 2);
    assert!(matches!(b.sent[0].body, IntentBody::DeclareMove { .. }));
    let refused = b.receipts.get(&1).expect("the move's receipt");
    assert_eq!(refused.len(), 1);
    assert!(
        matches!(refused[0], ReceiptV3::Rejected(_)),
        "a two-step move cannot afford the pool: {refused:?} sent={:?}",
        b.sent
    );
    assert_eq!(b.moves_applied(), 0, "only an applied DeclareMove counts");
    assert_eq!(b.receipts.get(&2), Some(&vec![ReceiptV3::Applied]));
}

/// B-6.6: the lane-1 sample, not the anchor, is what moves a walking replica
/// between its anchor positions.
#[test]
fn samples_move_the_replica_between_anchors() {
    let (hero, _goblin) = combatants();
    let seats = vec![Seat {
        name: "A",
        control: vec![hero],
        grants: Grants::Full,
        faults: Faults::CLEAN,
        script: Script::Walk {
            actor: n(1),
            points: [pt(65536, 0, 65536), pt(0, 0, 0)],
        },
        twin: false,
    }];
    let mut arena = Arena::new(EncounterMode::RealTime, Some(4), seats, NET_SEED, false);
    let mut seen: Vec<(Point, bool)> = Vec::new();
    for _ in 0..8 {
        arena.step();
        let rep = &arena.client("A").replica;
        assert_eq!(
            canon(rep).positions.get(&n(1)),
            rep.position(n(1)).map(|p| (p.point, p.moving)).as_ref(),
            "canon keeps the moving flag"
        );
        if let Some(state) = rep.position(n(1)) {
            let pair = (state.point, state.moving);
            if seen.last() != Some(&pair) {
                seen.push(pair);
            }
        }
    }
    assert!(
        seen.contains(&(pt(46340, 0, 46340), true)),
        "a lane-1 sample moves the replica between anchors: {seen:?}"
    );
    assert!(
        seen.contains(&(pt(65536, 0, 65536), false)),
        "the arrival sample reaches the destination: {seen:?}"
    );
    let rep = &arena.client("A").replica;
    assert_eq!(
        canon(rep).encounter,
        *rep.v3().encounter(),
        "canon keeps the encounter image"
    );
    assert_ne!(
        canon(rep).encounter,
        crpg_net::protocol_v3::EncounterFacts::default(),
        "a real-time encounter is not the default image"
    );
}

// ---------------------------------------------------------------------------
// Fix round B: the remaining B-7, Interface and script/canon gaps.
// ---------------------------------------------------------------------------

/// Interface: `in_flight` counts every channel, including `up` and the
/// datagram lane.
#[test]
fn in_flight_counts_all_four_channels() {
    let (hero, _goblin) = combatants();
    let seats = vec![Seat {
        name: "A",
        control: vec![hero],
        grants: Grants::Full,
        faults: Faults::CLEAN,
        script: Script::Idle,
        twin: false,
    }];
    let mut arena = Arena::new(EncounterMode::TurnBased, None, seats, NET_SEED, false);
    let a = arena.client_mut("A");
    a.channel_mut(Which::Up).send(0, vec![1], Faults::CLEAN);
    a.channel_mut(Which::Down0).send(0, vec![2], Faults::CLEAN);
    a.channel_mut(Which::Down1).send(0, vec![3], Faults::CLEAN);
    a.channel_mut(Which::Datagram)
        .send(0, vec![4], Faults::CLEAN);
    assert_eq!(a.in_flight(), 4, "one frame in each of the four channels");
    assert_eq!(a.channel_mut(Which::Down0).len(), 1);
    assert_eq!(a.channel_mut(Which::Down1).len(), 1);
}

/// Interface/B-18: `held_delta_bytes` reports the delta pages the gate is
/// holding, not a constant zero.
#[test]
fn held_delta_bytes_reports_a_buffered_page() {
    let (hero, goblin) = combatants();
    let seats = vec![
        Seat {
            name: "A",
            control: vec![hero],
            grants: Grants::Full,
            faults: Faults::CLEAN,
            script: Script::Fight {
                actor: n(1),
                target: n(2),
                first_move: pt(32768, 0, 32768),
            },
            twin: false,
        },
        Seat {
            name: "B",
            control: vec![goblin],
            grants: Grants::Full,
            faults: Faults::CLEAN,
            script: Script::Fight {
                actor: n(2),
                target: n(1),
                first_move: pt(32768, 0, 0),
            },
            twin: false,
        },
    ];
    let mut arena = Arena::new(EncounterMode::TurnBased, None, seats, NET_SEED, false);
    arena.run(3);
    assert_eq!(
        arena.client("A").held_delta_bytes(),
        0,
        "nothing is held before the gap"
    );
    arena.client_mut("A").channel_mut(Which::Down0).drop_next();
    let mut held = false;
    for _ in 0..100 {
        arena.step();
        if arena.client("A").held_delta_bytes() > 0 {
            held = true;
            break;
        }
    }
    assert!(
        held,
        "a dropped delta leaves the next page buffered behind its gap"
    );
}

/// B-8(b): a `Patrol` acts once per turn fact. Both sends from an
/// unauthorised seat are refused, and the unchanged fact never makes it act
/// again.
#[test]
fn patrol_acts_once_per_turn_fact() {
    let (_hero, _goblin) = combatants();
    let seats = vec![Seat {
        name: "O",
        control: Vec::new(),
        grants: Grants::Observer,
        faults: Faults::CLEAN,
        script: Script::Patrol {
            actor: n(2),
            points: [pt(32768, 0, 0), pt(0, 0, 0)],
        },
        twin: false,
    }];
    let mut arena = Arena::new(EncounterMode::TurnBased, None, seats, NET_SEED, false);
    arena.run(10);
    let o = arena.client("O");
    assert_eq!(
        o.sent.len(),
        2,
        "one move and one end turn for the single turn fact"
    );
    assert!(matches!(
        o.sent[0].body,
        IntentBody::DeclareMove { destination } if destination == pt(32768, 0, 0)
    ));
    assert_eq!(o.sent[0].actor, n(2));
    assert!(matches!(o.sent[1].body, IntentBody::EndTurn));
    assert_eq!(o.sent[1].actor, n(2));
    assert_eq!(
        o.receipts.get(&1),
        Some(&vec![ReceiptV3::Rejected(RejectionCode::NotAuthorized)]),
        "the move is refused"
    );
    assert_eq!(
        o.receipts.get(&2),
        Some(&vec![ReceiptV3::Rejected(RejectionCode::NotAuthorized)]),
        "the end turn is refused"
    );
    assert_eq!(o.moves_applied(), 0);
}

/// B-8(a): the end turn is conditioned on the receipt belonging to the turn
/// fact it moved under. A receipt that lands on a later, hero-owned turn ends
/// nothing.
#[test]
fn patrol_does_not_end_a_new_turn_for_an_old_receipt() {
    let (hero, goblin) = combatants();
    // `C` is first so its `EndTurn` on the hero lands while the goblin still
    // owns the turn and is refused; `B`'s end turn then hands the turn back to
    // the hero, so the hero's turn stays visible to `A` every cycle. `A`'s
    // own move is delayed past that turn, so its receipt lands on a later
    // hero turn fact.
    let seats = vec![
        Seat {
            name: "C",
            control: vec![hero],
            grants: Grants::Full,
            faults: Faults::CLEAN,
            script: Script::Intruder {
                actor: n(1),
                every: 1,
            },
            twin: false,
        },
        Seat {
            name: "A",
            control: vec![hero],
            grants: Grants::Full,
            faults: Faults {
                latency: 4,
                ..Faults::CLEAN
            },
            script: Script::Patrol {
                actor: n(1),
                points: [pt(32768, 0, 32768), pt(0, 0, 0)],
            },
            twin: false,
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
            twin: false,
        },
    ];
    let mut arena = Arena::new(EncounterMode::TurnBased, None, seats, NET_SEED, false);
    let mut acted = None;
    let mut at_receipt = None;
    for _ in 0..120 {
        let before = arena.client("A").replica.v3().turn().cloned();
        let sent_before = arena.client("A").sent.len();
        arena.step();
        let a = arena.client("A");
        if acted.is_none() && a.sent.len() > sent_before {
            acted = Some(before);
        }
        if acted.is_some() && a.receipts.contains_key(&1) {
            at_receipt = Some(a.replica.v3().turn().cloned());
            break;
        }
    }
    let acted = acted.expect("A sends its first move");
    let turn = at_receipt.expect("A's first move is receipted");
    assert_eq!(
        turn.clone().expect("a turn fact").active,
        Some(n(1)),
        "the delayed receipt lands on a later hero turn"
    );
    assert_ne!(turn, acted, "the receipt lands on a later turn fact");
    let sent = arena.client("A").sent.len();
    let bodies: Vec<_> = arena.client("A").sent[..sent]
        .iter()
        .map(|f| f.body)
        .collect();
    arena.step();
    let a = arena.client("A");
    assert!(
        !a.sent[sent..]
            .iter()
            .any(|f| matches!(f.body, IntentBody::EndTurn)),
        "an old receipt ends no later turn: {:?} then {:?}",
        bodies,
        &a.sent[sent..]
    );
    assert!(a
        .sent
        .iter()
        .all(|f| matches!(f.body, IntentBody::DeclareMove { .. } | IntentBody::EndTurn)));
}

/// B-9/B-12: a fight opens with its own first move, then strikes, the mirror
/// agrees after every step, and nobody acts once a row is dead.
#[test]
fn fight_moves_once_strikes_and_stops_at_death() {
    let (hero, goblin) = combatants();
    let seats = vec![
        Seat {
            name: "A",
            control: vec![hero],
            grants: Grants::Full,
            faults: Faults::CLEAN,
            script: Script::Fight {
                actor: n(1),
                target: n(2),
                first_move: pt(32768, 0, 32768),
            },
            twin: false,
        },
        Seat {
            name: "B",
            control: vec![goblin],
            grants: Grants::Full,
            faults: Faults::CLEAN,
            script: Script::Fight {
                actor: n(2),
                target: n(1),
                first_move: pt(32768, 0, 0),
            },
            twin: false,
        },
    ];
    let mut arena = Arena::new(EncounterMode::TurnBased, None, seats, NET_SEED, true);
    let mut dead_at = None;
    for step in 0..1_000 {
        arena.step();
        assert_eq!(
            arena.authority(),
            arena.mirror_hash(),
            "the mirror after step {step}"
        );
        if dead_at.is_none()
            && ["A", "B"].iter().any(|name| {
                arena
                    .client(name)
                    .replica
                    .v3()
                    .entities()
                    .iter()
                    .any(|row| row.health.as_ref().is_some_and(|h| h.dead))
            })
        {
            dead_at = Some((arena.client("A").sent.len(), arena.client("B").sent.len()));
        }
        if dead_at.is_some() {
            break;
        }
    }
    let (a_sent, b_sent) = dead_at.expect("the fight ends");
    let a = arena.client("A");
    let b = arena.client("B");
    assert!(matches!(
        a.sent[0].body,
        IntentBody::DeclareMove { destination } if destination == pt(32768, 0, 32768)
    ));
    assert!(matches!(
        b.sent[0].body,
        IntentBody::DeclareMove { destination } if destination == pt(32768, 0, 0)
    ));
    assert!(
        a.sent
            .iter()
            .any(|f| matches!(f.body, IntentBody::DeclareAction { ability, target } if ability == strike() && target == n(2))),
        "the hero strikes the goblin"
    );
    assert!(
        b.sent
            .iter()
            .any(|f| matches!(f.body, IntentBody::DeclareAction { ability, target } if ability == strike() && target == n(1))),
        "the goblin strikes the hero"
    );
    arena.run(20);
    assert_eq!(arena.client("A").sent.len(), a_sent, "A stops at the death");
    assert_eq!(arena.client("B").sent.len(), b_sent, "B stops at the death");
    assert_eq!(arena.authority(), arena.mirror_hash());
}

/// B-13: a lane-0 send goes out under the seat's own faults, not a clean
/// profile. With every reliable send losing `MAX_LOSSES` times, the answer
/// pays both directions' retransmission delays.
#[test]
fn lane_zero_send_pays_the_seat_faults() {
    let (hero, goblin) = combatants();
    let seats = vec![
        Seat {
            name: "A",
            control: vec![hero],
            grants: Grants::Full,
            faults: Faults::CLEAN,
            script: Script::Idle,
            twin: false,
        },
        Seat {
            name: "B",
            control: vec![goblin],
            grants: Grants::Full,
            faults: Faults {
                loss_percent: 100,
                ..Faults::CLEAN
            },
            script: Script::Patrol {
                actor: n(2),
                points: [pt(32768, 0, 0), pt(0, 0, 0)],
            },
            twin: false,
        },
    ];
    let mut arena = Arena::new(EncounterMode::TurnBased, None, seats, NET_SEED, false);
    let mut send_cycle = None;
    let mut receipt_cycle = None;
    for _ in 0..80 {
        let c = arena.cycle;
        arena.step();
        let b = arena.client("B");
        if send_cycle.is_none() && !b.sent.is_empty() {
            send_cycle = Some(c);
        }
        if receipt_cycle.is_none() {
            if let Some(&(_, at)) = b.receipt_cycles.iter().find(|(seq, _)| *seq == 1) {
                receipt_cycle = Some(at);
            }
        }
        if receipt_cycle.is_some() {
            break;
        }
    }
    let send = send_cycle.expect("B sends its move");
    let receipt = receipt_cycle.expect("the move's receipt arrives");
    assert!(
        receipt >= send + 2 * u64::from(MAX_LOSSES),
        "the intent and its receipt each pay MAX_LOSSES x rto: send {send}, receipt {receipt}"
    );
    assert_eq!(
        arena.client("B").receipts.get(&1),
        Some(&vec![ReceiptV3::Applied])
    );
}

/// B-11: a walk waits for a disclosed position, then for the previous intent
/// to clear, and only continues once the arrival sample has cleared `moving`.
#[test]
fn walk_gates_on_position_pending_and_motion() {
    let (hero, _goblin) = combatants();
    let seats = vec![Seat {
        name: "A",
        control: vec![hero],
        grants: Grants::Full,
        faults: Faults::CLEAN,
        script: Script::Walk {
            actor: n(1),
            points: [pt(65536, 0, 65536), pt(0, 0, 0)],
        },
        twin: false,
    }];
    let mut arena = Arena::new(EncounterMode::RealTime, Some(4), seats, NET_SEED, false);

    // P15: at cycle 0 the replica has no position, so the walk sends nothing.
    arena.step();
    assert!(
        arena.client("A").move_receipts.is_empty(),
        "no lane-1 intent before a position is disclosed"
    );

    // Cycle 1 discloses the position and sends the first leg.
    arena.step();
    assert_eq!(
        arena.client("A").outbox.pending().len(),
        0,
        "the clean receipt clears the first leg"
    );

    // P17: a position at the destination that is still moving is not an
    // arrival.
    let sample = MoveSample {
        server_tick: 1_000_000,
        processed_seq: 0,
        entries: vec![SampleEntry {
            entity: n(1),
            point: pt(65536, 0, 65536),
            moving: true,
        }],
    };
    let bytes = movement_v4::encode_down(&MoveDown::Sample(sample)).expect("a sample encodes");
    let now = arena.cycle;
    arena
        .client_mut("A")
        .channel_mut(Which::Datagram)
        .send(now, bytes, Faults::CLEAN);
    arena.step();
    {
        let a = arena.client("A");
        assert_eq!(
            a.replica.position(n(1)).map(|p| (p.point, p.moving)),
            Some((pt(65536, 0, 65536), true)),
            "the crafted sample is the newest position"
        );
        assert_eq!(a.outbox.pending().len(), 0);
    }
    arena.step();
    assert_eq!(
        arena.client("A").outbox.pending().len(),
        0,
        "a moving destination does not send the next leg"
    );

    // P16: an arrival with an intent already in flight does not send another.
    let sample = MoveSample {
        server_tick: 1_000_001,
        processed_seq: 0,
        entries: vec![SampleEntry {
            entity: n(1),
            point: pt(65536, 0, 65536),
            moving: false,
        }],
    };
    let bytes = movement_v4::encode_down(&MoveDown::Sample(sample)).expect("a sample encodes");
    let now = arena.cycle;
    arena
        .client_mut("A")
        .channel_mut(Which::Datagram)
        .send(now, bytes, Faults::CLEAN);
    arena.step();
    assert_eq!(
        arena
            .client("A")
            .replica
            .position(n(1))
            .map(|p| (p.point, p.moving)),
        Some((pt(65536, 0, 65536), false)),
        "the crafted arrival is the newest position"
    );
    arena
        .client_mut("A")
        .outbox
        .send(
            n(1),
            MoveBody::MoveTo {
                destination: pt(0, 0, 0),
            },
        )
        .expect("the outbox takes a pending leg");
    assert_eq!(arena.client("A").outbox.pending().len(), 1);
    arena.step();
    assert_eq!(
        arena.client("A").outbox.pending().len(),
        1,
        "a pending intent blocks the next leg"
    );
}

/// B-14: every `canon` field is the replica's, `world_view` strips only the
/// per-viewer order and host facts, and `authority` hashes the host.
#[test]
fn comparison_reports_the_contract_fields() {
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
            twin: false,
        },
    ];
    let mut arena = Arena::new(EncounterMode::TurnBased, None, seats, NET_SEED, true);
    arena.run(30);
    let rep = &arena.client("A").replica;
    let c = canon(rep);
    assert_eq!(c.entities, rep.v3().entities().to_vec(), "entities");
    assert_eq!(c.turn, rep.v3().turn().cloned(), "turn");
    assert_eq!(c.encounter, *rep.v3().encounter(), "encounter");
    assert_eq!(
        c.timeline,
        rep.v3().timeline().map(<[_]>::to_vec),
        "timeline"
    );
    assert_eq!(c.pause, rep.v3().pause(), "pause");
    assert!(c.pause.is_some(), "the clock is disclosed");
    assert_eq!(c.host_facts, rep.v3().host_facts().cloned(), "host facts");
    assert!(c.host_facts.is_some(), "the host facts are disclosed");
    assert_eq!(
        c.positions.get(&n(1)),
        rep.position(n(1)).map(|p| (p.point, p.moving)).as_ref(),
        "positions"
    );

    let view = world_view(rep);
    assert_eq!(view.host_facts, None);
    assert!(
        view.entities.iter().all(|row| row.facts.order.is_none()),
        "world_view strips every order"
    );
    assert_eq!(arena.twin("A").name, "A~");
    assert_eq!(arena.twin("A").peer.get(), 2);

    let host = arena.host_loop.hosting().host().authority_hash();
    assert_eq!(arena.authority(), host);
    arena
        .mirror
        .as_mut()
        .expect("a mirror")
        .world
        .tick_with_movement()
        .expect("the mirror ticks");
    assert_ne!(arena.mirror_hash(), host, "the mirror has moved on");
    assert_eq!(
        arena.authority(),
        host,
        "authority is the loop host's hash, never the mirror's"
    );
}
