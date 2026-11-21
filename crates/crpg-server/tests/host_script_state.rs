//! T052b (H1): a host saved to a file and reloaded mid-run keeps the
//! continuation, dialogue and quest state T052 added to `World`, and its
//! countdown continues as an uninterrupted host's (B-13, Reading 11).
//!
//! Notation (the contract's): `u(n)` is the ULID of 25 `0`s then the digit `n`;
//! `G = u(1)`, `W = u(2)`, `D = u(4)`, `N1 = u(5)`, `Q = u(7)`, `S1 = u(5)`;
//! `k(r)` is a continuation with `remaining = r`. `now_ms` is the manual
//! `50 * i` sequence; only `CARGO_TARGET_TMPDIR` is written.
//!
//! A host exposes no `World` getter, so every recorded state is read from a
//! quiescent `Host::save_checkpoint` (public, no file I/O), whose JSON inlines
//! the authoritative `HistoryWorld`. The file round trip itself is
//! `save_host_file` / `load_host_file` under `CARGO_TARGET_TMPDIR`.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crpg_core::{EntityId, Ulid};
use crpg_server::host::{Host, HostConfig, ProtocolSelection};
use crpg_server::save::{load_host_file, save_host_file, SaveIdentity};
use crpg_sim::{EntityMeta, HistoryWorld, OpenDialogue, ScriptContinuation};

const G: u8 = 1;
const W: u8 = 2;
const D: u8 = 4;
const N1: u8 = 5;
const Q: u8 = 7;
const S1: u8 = 5;

fn u(n: u8) -> Ulid {
    format!("{}{n}", "0".repeat(25))
        .parse()
        .expect("26 characters of a valid ULID")
}

fn k(remaining: u64) -> ScriptContinuation {
    ScriptContinuation {
        graph: u(G),
        node: u(W),
        remaining,
    }
}

fn dialogue() -> OpenDialogue {
    OpenDialogue {
        dialogue: u(D),
        node: u(N1),
    }
}

/// The fixture the existing `host_save.rs` uses: one protocol, incarnation 1.
fn config() -> HostConfig {
    HostConfig {
        protocol: ProtocolSelection::V1,
        incarnation: 1,
    }
}

fn identity() -> SaveIdentity {
    SaveIdentity {
        campaign_id: Ulid::from_u128(700),
        campaign_version: "1.0.0".to_owned(),
    }
}

/// The starting authority: a 600-tick wait, one participant mid-dialogue and
/// quest `Q` in state `S1`. The journal is fully acknowledged before the wrap.
struct Fixture {
    history: HistoryWorld,
    holder: EntityId,
    participant: EntityId,
}

fn fixture() -> Fixture {
    let mut history = HistoryWorld::new(7);
    let holder = history
        .spawn_continuation(k(600))
        .expect("a fresh journal has room");
    let participant = history
        .spawn(EntityMeta {})
        .expect("a fresh journal has room");
    history
        .open_dialogue(participant, dialogue())
        .expect("a live participant with none open");
    assert_eq!(history.set_quest_state(u(Q), u(S1)), Ok(None));
    history
        .acknowledge(history.last_sequence())
        .expect("acknowledges its own range");
    Fixture {
        history,
        holder,
        participant,
    }
}

/// `CARGO_TARGET_TMPDIR/crpg-server/host_script_state`, emptied first.
fn case_dir() -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("crpg-server")
        .join("host_script_state");
    match fs::remove_dir_all(&dir) {
        Ok(()) => {}
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => panic!("cannot clear {}: {e}", dir.display()),
    }
    fs::create_dir_all(&dir).expect("creates the case directory");
    dir
}

/// The authoritative `HistoryWorld` inside `host`'s own quiescent checkpoint.
fn authority(host: &Host) -> HistoryWorld {
    let checkpoint = host.save_checkpoint().expect("a quiescent host");
    let value: serde_json::Value =
        serde_json::from_slice(&checkpoint).expect("a checkpoint is compact JSON");
    let history = value
        .get("history")
        .expect("a checkpoint carries its history");
    serde_json::from_value(history.clone()).expect("the history loads")
}

/// One advance for each host at `50 * i`, then the equivalence assertions.
fn step(a: &mut Host, b: &mut Host, i: u64) {
    a.advance(50 * i).expect("time only moves forward");
    b.advance(50 * i).expect("time only moves forward");
    assert_eq!(a.server_tick(), i, "step {i}");
    assert_eq!(b.server_tick(), i, "step {i}");
    assert_eq!(
        b.authority_hash(),
        a.authority_hash(),
        "step {i}: the authority is identical"
    );
}

/// `save_host_file`, drop, `load_host_file` under incarnation 2 at `now_ms`,
/// then the restored host's unpause.
fn restart(host: Host, path: &Path, now_ms: u64) -> Host {
    save_host_file(path, &host, &identity()).expect("quiescent save");
    let saved = host.authority_hash();
    drop(host);
    let mut loaded = load_host_file(path, &identity(), 2, now_ms)
        .expect("the fresh save loads")
        .host;
    assert_eq!(
        loaded.authority_hash(),
        saved,
        "the load keeps the authority"
    );
    loaded.unpause(now_ms).expect("a restored host is paused");
    loaded
}

/// The three kinds of state at `remaining` ticks left.
fn assert_state(host: &Host, fixture: &Fixture, remaining: u64, at: &str) {
    let history = authority(host);
    let world = history.world();
    assert_eq!(
        world.dialogues().get(fixture.participant),
        Some(&dialogue()),
        "{at}: the participant's dialogue"
    );
    assert_eq!(
        world.quest_state(u(Q)),
        Some(u(S1)),
        "{at}: the quest's state"
    );
    assert_eq!(
        world.continuations().get(fixture.holder).copied(),
        Some(k(remaining)),
        "{at}: the continuation's resume point"
    );
}

#[test]
fn restart_keeps_script_state() {
    let fixture = fixture();

    // A runs unbroken; B is saved at step 500 and reloaded.
    let mut a = Host::from_history(fixture.history.clone(), config(), 0)
        .expect("an acknowledged authority wraps");
    let mut b = Host::from_history(fixture.history.clone(), config(), 0)
        .expect("an acknowledged authority wraps");
    a.unpause(0).expect("running already");
    b.unpause(0).expect("running already");

    for i in 1..=500 {
        step(&mut a, &mut b, i);
    }
    let dir = case_dir();
    let mut b = restart(b, &dir.join("mid.save"), 25_000);
    // Step 500 left the 600-tick wait at 100: the records survived the file.
    assert_state(&b, &fixture, 100, "after the reload");

    for i in 501..=1000 {
        step(&mut a, &mut b, i);
        if i == 599 || i == 600 {
            // First due exactly at server tick 600: 1 left at 599, 0 at 600.
            let remaining = if i == 599 { 1 } else { 0 };
            let due = if i == 599 {
                Vec::new()
            } else {
                vec![fixture.holder]
            };
            for host in [&a, &b] {
                let history = authority(host);
                let world = history.world();
                assert_eq!(
                    world
                        .continuations()
                        .get(fixture.holder)
                        .map(|c| c.remaining),
                    Some(remaining),
                    "step {i}: the countdown continued"
                );
                assert_eq!(
                    world.due_continuations(),
                    due,
                    "step {i}: due only from tick 600"
                );
            }
        }
    }

    assert_eq!(a.server_tick(), 1000);
    assert_eq!(b.server_tick(), 1000);
    assert_eq!(
        b.authority_hash(),
        a.authority_hash(),
        "the final authority is identical"
    );
    for host in [&a, &b] {
        assert_state(host, &fixture, 0, "at the end");
        let history = authority(host);
        let world = history.world();
        assert_eq!(
            world.due_continuations(),
            vec![fixture.holder],
            "the due holder is never removed"
        );
    }

    // Control: the save point does not change the final hash. A run saved at
    // step 599 and continued to 1000 ends where both others end.
    let mut control = Host::from_history(fixture.history.clone(), config(), 0)
        .expect("an acknowledged authority wraps");
    control.unpause(0).expect("running already");
    for i in 1..=599 {
        control.advance(50 * i).expect("time only moves forward");
    }
    let mut control = restart(control, &dir.join("control.save"), 29_950);
    for i in 600..=1000 {
        control.advance(50 * i).expect("time only moves forward");
    }
    assert_eq!(control.server_tick(), 1000);
    assert_eq!(
        control.authority_hash(),
        a.authority_hash(),
        "a save at step 599 gives the same final hash"
    );
}
