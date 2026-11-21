//! T041b acceptance suite (`tasks/T041.md`, D1–D5): the 5,000-tick loss and
//! jitter desync, seeded repeatability, the lost-frame oracle, lane 1 under
//! loss and the gate-13 embedded two-client fight, all through
//! `tests/support/headless.rs`.
//!
//! Expected values come from the contract, never from running the code. Each
//! case carries the control the contract names; every checkpoint names the
//! block or phase it failed in.

#[path = "support/headless.rs"]
mod headless;

use headless::*;

use crpg_core::EntityId;
use crpg_net::movement_v4::{self, MoveDown, MoveOutcome, MoveSample, SampleEntry};
use crpg_net::protocol_transfer::HealthFact;
use crpg_net::protocol_v3::{ReceiptStatus as ReceiptV3, RejectionCode as CodeV3};
use crpg_net::protocol_v4::{IntentBody, IntentFrame};
use crpg_sim::EncounterMode;

/// The started `combat_basic` combatants: hero first, goblin second.
fn combatants() -> (EntityId, EntityId) {
    let start = headless::start(EncounterMode::TurnBased, None);
    (start.hero, start.goblin)
}

/// Layout L1 (B-16): A patrols the hero, B patrols the goblin, O intrudes;
/// every seat has a faultless twin.
fn l1(hero: EntityId, goblin: EntityId) -> Vec<Seat> {
    vec![
        Seat {
            name: "A",
            control: vec![hero],
            grants: Grants::Full,
            faults: Faults::SPEC,
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
            faults: Faults::SPEC,
            script: Script::Patrol {
                actor: n(2),
                points: [pt(32768, 0, 0), pt(0, 0, 0)],
            },
            twin: true,
        },
        Seat {
            name: "O",
            control: Vec::new(),
            grants: Grants::Observer,
            faults: Faults::SPEC,
            script: Script::Intruder {
                actor: n(1),
                every: 250,
            },
            twin: true,
        },
    ]
}

/// Layout L4 (B-19): A and B walk their own combatant on lane 1; both have a
/// twin and no mirror runs.
fn l4(hero: EntityId, goblin: EntityId) -> Vec<Seat> {
    vec![
        Seat {
            name: "A",
            control: vec![hero],
            grants: Grants::Full,
            faults: Faults::SPEC,
            script: Script::Walk {
                actor: n(1),
                points: [pt(65536, 0, 65536), pt(0, 0, 0)],
            },
            twin: true,
        },
        Seat {
            name: "B",
            control: vec![goblin],
            grants: Grants::Full,
            faults: Faults::SPEC,
            script: Script::Walk {
                actor: n(2),
                points: [pt(65536, 0, 0), pt(0, 0, 0)],
            },
            twin: true,
        },
    ]
}

/// Layout L5 (B-20): the two players move once and fight to the death over a
/// clean network; no twins.
fn l5(hero: EntityId, goblin: EntityId) -> Vec<Seat> {
    vec![
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
    ]
}

/// B-15 C1: every seat with a twin compares equal to it after a settle.
fn assert_twins_equal(arena: &Arena, names: &[&str], at: &str) {
    for name in names {
        assert_eq!(
            canon(&arena.client(name).replica),
            canon(&arena.twin(name).replica),
            "C1 {name} differs from its twin at {at}"
        );
    }
}

/// B-15 C2: the two players see the same world.
fn assert_players_agree(arena: &Arena, at: &str) {
    assert_eq!(
        world_view(&arena.client("A").replica),
        world_view(&arena.client("B").replica),
        "C2 A and B see different worlds at {at}"
    );
}

/// B-15 C3: the authority equals the independent mirror.
fn assert_authority_matches_mirror(arena: &Arena, at: &str) {
    assert_eq!(
        arena.authority(),
        arena.mirror_hash(),
        "C3 the authority and the mirror differ at {at}"
    );
}

/// B-15 C4: no gate or replica refusal, and no held delta page.
fn assert_no_refusals(arena: &Arena, at: &str) {
    for client in &arena.clients {
        assert!(
            client.gate_errors.is_empty(),
            "C4 {} gate errors at {at}: {:?}",
            client.name,
            client.gate_errors
        );
        assert!(
            client.replica_errors.is_empty(),
            "C4 {} replica errors at {at}: {:?}",
            client.name,
            client.replica_errors
        );
        assert_eq!(
            client.held_delta_bytes(),
            0,
            "C4 {} holds delta bytes at {at}",
            client.name
        );
    }
}

/// B-15 C5: every lane-0 seq the client sent has exactly one receipt.
fn assert_one_receipt_per_intent(arena: &Arena, at: &str) {
    for client in &arena.clients {
        for frame in &client.sent {
            assert_eq!(
                client.receipts.get(&frame.seq).map(Vec::len),
                Some(1),
                "C5 {} seq {} has no single receipt at {at}",
                client.name,
                frame.seq
            );
        }
    }
}

/// B-15: the whole checkpoint, after the caller's `settle()`.
fn checkpoint(arena: &Arena, at: &str) {
    assert_twins_equal(arena, &["A", "B", "O"], at);
    assert_players_agree(arena, at);
    assert_authority_matches_mirror(arena, at);
    assert_no_refusals(arena, at);
    assert_one_receipt_per_intent(arena, at);
}

/// Every lane-0 receipt of `name` is `[Applied]` (B-16) or the named code.
fn assert_every_receipt_is(arena: &Arena, name: &str, expected: ReceiptV3, at: &str) {
    let client = arena.client(name);
    for (seq, statuses) in &client.receipts {
        assert_eq!(
            statuses,
            &vec![expected],
            "{name} seq {seq} has an unexpected receipt at {at}"
        );
    }
    assert_eq!(
        client.receipts.len(),
        client.sent.len(),
        "{name} sent {} intents but saw {} receipted seqs at {at}",
        client.sent.len(),
        client.receipts.len()
    );
}

/// D1's checkpoint counts, for the as-built documentation.
///
/// Not a contract item: the doc records D1's measured counts (moves per
/// player, intrusions, losses, duplicates). Keeping the measurement beside
/// the run means the numbers in the architecture doc have a source.
struct Counts {
    a_moves: u64,
    b_moves: u64,
    intrusions: usize,
    reliable_lost: u64,
    datagram_lost: u64,
    datagram_duplicated: u64,
}

/// Reads D1's counts off the finished arena (B-16's floors, printed).
fn counts(arena: &mut Arena) -> Counts {
    let mut reliable_lost = 0;
    let mut datagram_lost = 0;
    let mut datagram_duplicated = 0;
    for name in ["A", "B", "O"] {
        for which in [Which::Up, Which::Down0, Which::Down1] {
            reliable_lost += arena.client_mut(name).channel_mut(which).stats().lost;
        }
        let datagram = arena.client_mut(name).channel_mut(Which::Datagram).stats();
        datagram_lost += datagram.lost;
        datagram_duplicated += datagram.duplicated;
    }
    Counts {
        a_moves: arena.client("A").moves_applied(),
        b_moves: arena.client("B").moves_applied(),
        intrusions: arena.client("O").sent.len(),
        reliable_lost,
        datagram_lost,
        datagram_duplicated,
    }
}

/// D1: five blocks of 1,000 faulted cycles, each checkpointed after a settle.
#[test]
fn d1_five_thousand_ticks_under_loss_and_jitter() {
    let (hero, goblin) = combatants();
    let mut arena = Arena::new(
        EncounterMode::TurnBased,
        None,
        l1(hero, goblin),
        NET_SEED,
        true,
    );
    assert_eq!(arena.clients.len(), 6, "L1 binds three clients and twins");
    for (index, client) in arena.clients.iter().enumerate() {
        assert_eq!(
            client.peer.get(),
            index as u64 + 1,
            "peers ascend with the vector index"
        );
    }
    for block in 0..5u64 {
        for step in 0..1_000u64 {
            arena.step();
            if (step + 1).is_multiple_of(100) {
                assert_eq!(
                    arena.authority(),
                    arena.mirror_hash(),
                    "the mirror after block {block} step {}",
                    step + 1
                );
            }
        }
        arena.settle();
        checkpoint(&arena, &format!("block {block}"));
        arena.resume();
    }

    for name in ["A", "B"] {
        assert_every_receipt_is(&arena, name, ReceiptV3::Applied, "the final block");
    }
    assert_every_receipt_is(
        &arena,
        "O",
        ReceiptV3::Rejected(CodeV3::NotAuthorized),
        "the final block",
    );
    assert_eq!(
        arena.host_loop.hosting().host().executions(),
        (arena.client("A").sent.len() + arena.client("B").sent.len()) as u64,
        "every accepted lane-0 command was executed exactly once"
    );
    assert!(
        arena.client("A").moves_applied() >= 100,
        "A applied only {} moves",
        arena.client("A").moves_applied()
    );
    assert!(
        arena.client("B").moves_applied() >= 100,
        "B applied only {} moves",
        arena.client("B").moves_applied()
    );
    assert!(
        arena.client("O").sent.len() >= 15,
        "O sent only {} intrusions",
        arena.client("O").sent.len()
    );

    let counted = counts(&mut arena);
    assert!(
        counted.reliable_lost >= 1,
        "no reliable channel lost an attempt: {:?}",
        counted.reliable_lost
    );
    assert!(
        counted.datagram_lost >= 1,
        "no datagram was lost: {:?}",
        counted.datagram_lost
    );
    assert!(
        counted.datagram_duplicated >= 1,
        "no datagram was duplicated: {:?}",
        counted.datagram_duplicated
    );

    // The doc's as-built counts (task T041b "the as-built D1 counts"). The
    // assertion is the floor the contract sets; the numbers are printed for
    // the hand-off and written down in docs/architecture/crpg-testkit.md.
    println!(
        "D1 counts: A moves {} B moves {} intrusions {} reliable lost {} datagram lost {} duplicated {}",
        counted.a_moves,
        counted.b_moves,
        counted.intrusions,
        counted.reliable_lost,
        counted.datagram_lost,
        counted.datagram_duplicated
    );
}

/// One run's comparable record (B-17).
struct Record {
    /// The authority hash after every 100th step.
    hashes: Vec<[u8; 32]>,
    /// Every client's `sent`, in client order.
    sent: Vec<Vec<IntentFrame>>,
    /// Every client's `receipt_cycles`, in client order.
    receipt_cycles: Vec<Vec<(u64, u64)>>,
}

/// B-17: one 1,000-step L1 run under `net_seed`, recorded.
fn record_run(net_seed: u64) -> Record {
    let (hero, goblin) = combatants();
    let mut arena = Arena::new(
        EncounterMode::TurnBased,
        None,
        l1(hero, goblin),
        net_seed,
        true,
    );
    let mut hashes = Vec::new();
    for step in 0..1_000u64 {
        arena.step();
        if (step + 1).is_multiple_of(100) {
            hashes.push(arena.authority());
        }
    }
    Record {
        hashes,
        sent: arena.clients.iter().map(|c| c.sent.clone()).collect(),
        receipt_cycles: arena
            .clients
            .iter()
            .map(|c| c.receipt_cycles.clone())
            .collect(),
    }
}

/// D2: two runs with one seed repeat exactly; a different seed moves A's
/// receipt cycles.
#[test]
fn d2_seeded_runs_repeat() {
    let first = record_run(NET_SEED);
    let second = record_run(NET_SEED);
    assert_eq!(first.hashes.len(), 10, "one authority hash per 100 steps");
    assert_eq!(first.hashes, second.hashes, "authority hashes repeat");
    assert_eq!(first.sent, second.sent, "every client's sent frames repeat");
    assert_eq!(
        first.receipt_cycles, second.receipt_cycles,
        "every client's receipt cycles repeat"
    );

    // Control: NET_SEED + 1 changes the network, so A's receipt cycles differ.
    let other = record_run(NET_SEED + 1);
    assert_eq!(other.hashes.len(), 10);
    assert_ne!(
        first.receipt_cycles[0], other.receipt_cycles[0],
        "a different seed must reach A's receipt cycles"
    );
}

/// D3: one down-0 frame lost for good is caught by the twin oracle; B and O
/// stay converged in the same run.
#[test]
fn d3_a_frame_lost_for_good_is_caught() {
    let (hero, goblin) = combatants();
    let mut arena = Arena::new(
        EncounterMode::TurnBased,
        None,
        l1(hero, goblin),
        NET_SEED,
        true,
    );
    arena.run(150);
    arena.client_mut("A").channel_mut(Which::Down0).drop_next();
    arena.run(150);
    arena.settle();

    let a = arena.client("A");
    assert_ne!(
        canon(&a.replica),
        canon(&arena.twin("A").replica),
        "the oracle passed a client that missed a page"
    );
    assert!(
        a.held_delta_bytes() > 0 || !a.gate_errors.is_empty(),
        "the lost frame left no trace: held {} errors {:?}",
        a.held_delta_bytes(),
        a.gate_errors
    );
    assert_twins_equal(&arena, &["B", "O"], "D3's control");
    assert_authority_matches_mirror(&arena, "D3");
}

/// D4: both walkers keep lane-1 receipts exactly once, in order, accepted,
/// and converge with their twins; at least twenty legs are walked.
#[test]
fn d4_lane_one_walks_under_loss() {
    let (hero, goblin) = combatants();
    let mut arena = Arena::new(
        EncounterMode::RealTime,
        Some(4),
        l4(hero, goblin),
        NET_SEED,
        false,
    );
    arena.run(1_000);
    arena.settle();

    assert_twins_equal(&arena, &["A", "B"], "D4");
    assert_players_agree(&arena, "D4");
    assert_no_refusals(&arena, "D4");

    for name in ["A", "B"] {
        let client = arena.client(name);
        let k = client.move_receipts.len() as u64;
        let seqs: Vec<u64> = client.move_receipts.iter().map(|r| r.seq).collect();
        assert_eq!(
            seqs,
            (1..=k).collect::<Vec<u64>>(),
            "{name} lane-1 receipts are not exactly 1..={k}"
        );
        for receipt in &client.move_receipts {
            assert!(
                matches!(receipt.outcome, MoveOutcome::Accepted { .. }),
                "{name} seq {} was {:?}, not accepted",
                receipt.seq,
                receipt.outcome
            );
        }
        assert!(k >= 20, "{name} walked only {k} legs under loss and jitter");
    }
}

/// D5: the gate-13 embedded smoke — two clients move, fight to one death and
/// agree with each other and with the mirror.
#[test]
fn d5_two_clients_move_fight_and_agree() {
    let (hero, goblin) = combatants();
    let mut arena = Arena::new(
        EncounterMode::TurnBased,
        None,
        l5(hero, goblin),
        NET_SEED,
        true,
    );
    let mut ended = None;
    for step in 0..1_000u64 {
        arena.step();
        assert_eq!(
            arena.authority(),
            arena.mirror_hash(),
            "the mirror after step {step}"
        );
        let any_dead = |name: &str| {
            arena
                .client(name)
                .replica
                .v3()
                .entities()
                .iter()
                .any(|row| row.health.as_ref().is_some_and(|h| h.dead))
        };
        if any_dead("A") && any_dead("B") {
            ended = Some(step);
            break;
        }
    }
    let ended = ended.expect("fight");
    println!("D5: both players saw the death by step {ended}");
    arena.run(20);
    assert_eq!(arena.authority(), arena.mirror_hash(), "after the extra 20");

    assert_players_agree(&arena, "D5");
    let dead_rows: Vec<_> = arena
        .client("A")
        .replica
        .v3()
        .entities()
        .iter()
        .filter(|row| row.health.as_ref().is_some_and(|h| h.dead))
        .collect();
    assert_eq!(dead_rows.len(), 1, "exactly one row is dead");
    let dead_id = dead_rows[0].net_id;

    let combatants = arena
        .mirror
        .as_ref()
        .expect("a mirror")
        .world
        .world()
        .combatants();
    for (net_id, entity) in [(n(1), hero), (n(2), goblin)] {
        let combatant = combatants.get(entity).expect("a mirrored combatant");
        let expected = HealthFact {
            health: combatant.health(),
            max_health: combatant.max_health(),
            dead: combatant.dead(),
        };
        for name in ["A", "B"] {
            let row = arena
                .client(name)
                .replica
                .v3()
                .entities()
                .iter()
                .find(|row| row.net_id == net_id)
                .expect("a disclosed row");
            assert_eq!(
                row.health,
                Some(expected.clone()),
                "{name}'s row for {net_id:?} disagrees with the mirror"
            );
        }
    }

    // Control: the dead row's health is not its pinned start value.
    let start = match dead_id {
        id if id == n(1) => HealthFact {
            health: 10,
            max_health: 10,
            dead: false,
        },
        id if id == n(2) => HealthFact {
            health: 6,
            max_health: 6,
            dead: false,
        },
        other => panic!("unexpected dead row {other:?}"),
    };
    let dead_health = dead_rows[0].health.clone().expect("a dead row's health");
    assert_ne!(
        dead_health, start,
        "the dead row still carries its start health"
    );

    let applied = |name: &str, want_move: bool| {
        let client = arena.client(name);
        client
            .sent
            .iter()
            .filter(|frame| {
                let is_move = matches!(frame.body, IntentBody::DeclareMove { .. });
                is_move == want_move
                    && client.receipts.get(&frame.seq) == Some(&vec![ReceiptV3::Applied])
            })
            .count()
    };
    for name in ["A", "B"] {
        assert!(
            applied(name, true) >= 1,
            "{name} never made an applied DeclareMove"
        );
        assert!(
            applied(name, false) >= 1,
            "{name} never made an applied DeclareAction"
        );
    }
    assert_no_refusals(&arena, "D5");
}

/// Extra case (manager, T041b): B-11's `!p.moving` gate is load-bearing.
///
/// T041a's mutant P17 replaced `&& !p.moving` with `&& true` and survived
/// that suite, because `outbox.pending()` is already empty by the end of the
/// step in which the receipt arrives even when the walker sends the next leg
/// early. A destination that is still moving is not an arrival, so the next
/// step must send no leg at all: its receipt would be seq 2.
#[test]
fn walk_waits_while_moving() {
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
    arena.step();
    assert!(
        arena.client("A").move_receipts.is_empty(),
        "no lane-1 intent before a position is disclosed"
    );
    arena.step();
    {
        let a = arena.client("A");
        assert_eq!(a.move_receipts.len(), 1, "one leg for the first point");
        assert_eq!(a.move_receipts[0].seq, 1);
        assert!(matches!(
            a.move_receipts[0].outcome,
            MoveOutcome::Accepted { .. }
        ));
    }

    // The walker is at its leg-1 destination but the sample still says moving.
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
    assert_eq!(
        arena
            .client("A")
            .replica
            .position(n(1))
            .map(|p| (p.point, p.moving)),
        Some((pt(65536, 0, 65536), true)),
        "the crafted sample is the newest position"
    );

    // B-11: the next step sends no leg while the destination is still moving.
    arena.step();
    let a = arena.client("A");
    assert_eq!(
        a.move_receipts.iter().map(|r| r.seq).collect::<Vec<u64>>(),
        vec![1],
        "a moving destination must not send the next leg"
    );
    assert!(
        a.outbox.pending().is_empty(),
        "no second intent is in flight"
    );
    assert_eq!(
        a.replica.position(n(1)).map(|p| (p.point, p.moving)),
        Some((pt(65536, 0, 65536), true)),
        "the position is unchanged by the paused leg"
    );
}
