//! T041d acceptance suite (`tasks/T041.md` B-26 to B-32): the gate-13
//! dedicated smoke.
//!
//! One test starts the real `crpg-server` binary (`CARGO_BIN_EXE_crpg-server`,
//! T040b) over a directory of files made from the shared rig's fixture host,
//! connects two real QUIC clients, lets them fight over the combat wire, checks
//! that a rejected message appears as the binary's log line and is not applied,
//! compares both replicas with each other and with an independent mirror
//! authority, then quits the server and checks its exit, its save and its log.
//!
//! The binary paces itself by the wall clock, so every wait here is a guard
//! that panics `"deadline"` after `rig::DEADLINE` of real time, never an oracle
//! (`tasks/T041.md`, Reading 16). No credential byte appears in any message.

#[path = "support/quic_host_rig.rs"]
mod rig;

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use crpg_net::protocol::{ReceiptStatus, RejectionCode};
use crpg_net::protocol_v2::DeltaOp;
use crpg_net_quic::{ClientLimits, Hello, QuicClient};
use crpg_server::save::{load_host_file, save_host_file, SaveIdentity};
use crpg_sim::{CombatAction, HistoryWorld};
use rig::{Frame, Replica, State};
use serde_json::{json, Value};

const BIN: &str = env!("CARGO_BIN_EXE_crpg-server");
const CAMPAIGN: &str = "00000000000000000000000001";

/// `ID`: the campaign identity of the set-up's save.
fn id() -> SaveIdentity {
    SaveIdentity {
        campaign_id: CAMPAIGN.parse().expect("a campaign id"),
        campaign_version: "1.0.0".to_owned(),
    }
}

/// The 64 hex digits of the credential made of 32 bytes `byte`.
fn credential_hex(byte: u8) -> String {
    format!("{byte:02x}").repeat(32)
}

// ---------------------------------------------------------------------------
// B-26: the directory of files.
// ---------------------------------------------------------------------------

/// `rig::full_grants()` as the invitations file spells it, field for field.
fn grants_json() -> Value {
    let grants = rig::full_grants();
    let entities: Vec<Value> = grants
        .entities
        .iter()
        .map(|d| {
            json!({
                "entity": serde_json::to_value(d.entity).expect("entity json"),
                "present": d.present, "health": d.health, "spawn": d.spawn,
                "despawn": d.despawn, "died": d.died, "action_actor": d.action_actor,
                "action_target": d.action_target, "action_outcome": d.action_outcome,
                "action_damage": d.action_damage, "turn_start": d.turn_start
            })
        })
        .collect();
    json!({
        "entities": entities,
        "abilities": grants.abilities,
        "encounters": grants.encounters,
        "turn_round": grants.turn_round,
        "encounter_end": grants.encounter_end,
        "turn_state": grants.turn_state
    })
}

/// One entry of the invitations file.
fn invitation_json(byte: u8, control: &[crpg_core::EntityId]) -> Value {
    let control: Vec<Value> = control
        .iter()
        .map(|e| serde_json::to_value(e).expect("entity json"))
        .collect();
    json!({
        "credential": credential_hex(byte),
        "control": control,
        "grants": grants_json()
    })
}

/// B-26: a fresh `dir` holding the certificate, key, save, sidecar (`7\n`),
/// invitations (two entries) and configuration.
fn setup(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).expect("creates the directory");
    std::fs::write(dir.join("server.cert.der"), rig::SERVER_A_CERT).expect("cert");
    std::fs::write(dir.join("server.key.pk8.der"), rig::SERVER_A_KEY).expect("key");
    save_host_file(&dir.join("world.save"), &rig::new_host(rig::V2), &id())
        .expect("saves the fixture host");
    std::fs::write(dir.join("world.save.incarnation"), "7\n").expect("sidecar");
    let invitations = json!([
        invitation_json(0x11, &[rig::a(), rig::c()]),
        invitation_json(0x22, &[rig::b()])
    ]);
    std::fs::write(
        dir.join("invitations.json"),
        serde_json::to_vec(&invitations).expect("json"),
    )
    .expect("invitations");
    let config = json!({
        "listen": "127.0.0.1:0",
        "certificate": "server.cert.der",
        "private_key": "server.key.pk8.der",
        "invitations": "invitations.json",
        "save": "world.save",
        "campaign_id": CAMPAIGN,
        "campaign_version": "1.0.0"
    });
    std::fs::write(
        dir.join("server.json"),
        serde_json::to_vec(&config).expect("json"),
    )
    .expect("config");
}

// ---------------------------------------------------------------------------
// B-27: the process under test.
// ---------------------------------------------------------------------------

/// The running binary: its stdin, and its stderr lines forwarded by one
/// reader thread. Dropping it kills and reaps the child, so a failed run
/// leaves no process behind.
struct Server {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    port: u16,
    /// Every line read so far, in order.
    seen: Vec<String>,
}

impl Server {
    /// B-27: spawns the binary over `dir/server.json` and reads its start line.
    fn spawn(dir: &Path) -> Server {
        let mut child = Command::new(BIN)
            .arg("--config")
            .arg(dir.join("server.json"))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawns the server");
        let stdin = child.stdin.take().expect("piped stdin");
        let stderr = child.stderr.take().expect("piped stderr");
        let (send, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines() {
                let Ok(line) = line else { break };
                if send.send(line).is_err() {
                    break;
                }
            }
        });
        let mut server = Server {
            child,
            stdin,
            lines,
            port: 0,
            seen: Vec::new(),
        };
        let first = server.next_line("the start line");
        let port: u16 = first
            .strip_prefix("crpg-server: start listen=127.0.0.1:")
            .and_then(|rest| rest.split(' ').next())
            .and_then(|port| port.parse().ok())
            .unwrap_or_else(|| panic!("expected the start line, got {first:?}"));
        assert_eq!(
            first,
            format!(
                "crpg-server: start listen=127.0.0.1:{port} incarnation=8 pin={} nav=none ai=off invitations=2",
                rig::SERVER_A_PIN_HEX
            )
        );
        server.port = port;
        server
    }

    /// The next standard error line, waiting at most `rig::DEADLINE`.
    fn next_line(&mut self, what: &str) -> String {
        match self.lines.recv_timeout(rig::DEADLINE) {
            Ok(line) => {
                self.seen.push(line.clone());
                line
            }
            Err(RecvTimeoutError::Timeout) => panic!("deadline: {what}"),
            Err(RecvTimeoutError::Disconnected) => panic!("standard error closed before {what}"),
        }
    }

    /// B-32: writes `quit`, reads to the end of standard error and waits for
    /// the exit. Returns the status and every line of the run.
    fn quit(mut self) -> (ExitStatus, Vec<String>) {
        self.stdin.write_all(b"quit\n").expect("writes stdin");
        self.stdin.flush().expect("flushes stdin");
        loop {
            match self.lines.recv_timeout(rig::DEADLINE) {
                Ok(line) => self.seen.push(line),
                Err(RecvTimeoutError::Timeout) => panic!("deadline: the end of standard error"),
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        let start = Instant::now();
        let status = loop {
            if let Some(status) = self.child.try_wait().expect("try_wait") {
                break status;
            }
            assert!(start.elapsed() < rig::DEADLINE, "deadline: the exit");
            std::thread::sleep(Duration::from_millis(10));
        };
        (status, std::mem::take(&mut self.seen))
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ---------------------------------------------------------------------------
// B-28, B-30: the players.
// ---------------------------------------------------------------------------

/// One real QUIC client with its v2 replica (`rig::Frame`, `rig::Replica`).
struct Player {
    quic: QuicClient,
    epoch: [u8; 16],
    peer: u64,
    frames: Vec<Frame>,
    next_seq: u64,
    /// The `(active, round)` turn the player has already acted on.
    acted: Option<(Option<u64>, u64)>,
    /// `(seq, status, processed_tick)` of every receipt, in arrival order.
    receipts: Vec<(u64, ReceiptStatus, u64)>,
    /// The replica ids the player controls, and the one it strikes.
    own: Vec<u64>,
    target: u64,
    /// `seq` to `(actor, target)` of every declaration sent.
    declared: BTreeMap<u64, (u64, u64)>,
    /// How many receipts `react` has handled.
    handled: usize,
    /// `StaleTick` re-sends in the current turn, and in all.
    stale_in_turn: u32,
    stale_total: u32,
}

impl Player {
    /// B-28: connects with the credential of 32 bytes `byte` and checks the
    /// welcome's epoch: incarnation 8 in its first half, `peer` in its second.
    fn connect(port: u16, byte: u8, peer: u64, own: &[u64], target: u64) -> Player {
        let hello = Hello {
            wire_version: 2,
            credential: rig::credential(&[byte; 32]),
        };
        let server = SocketAddr::from(([127, 0, 0, 1], port));
        let quic = QuicClient::connect(rig::client_config(ClientLimits::v1()), server, &hello)
            .unwrap_or_else(|error| panic!("player {peer} cannot connect: {error:?}"));
        let epoch = quic.welcome().epoch;
        let player = Player {
            quic,
            epoch,
            peer: u64::from_le_bytes(epoch[8..16].try_into().expect("eight bytes")),
            frames: Vec::new(),
            next_seq: 1,
            acted: None,
            receipts: Vec::new(),
            own: own.to_vec(),
            target,
            declared: BTreeMap::new(),
            handled: 0,
            stale_in_turn: 0,
            stale_total: 0,
        };
        assert_eq!(player.epoch[..8], 8u64.to_le_bytes(), "incarnation 8");
        assert_eq!(player.peer, peer, "the peer handle");
        player
    }

    /// Reads every frame available now, as `rig::Client::read_available`.
    fn read(&mut self) {
        loop {
            match self.quic.try_recv() {
                Ok(Some(bytes)) => {
                    let frame = rig::decode_frame(rig::V2, &bytes);
                    for op in &frame.ops {
                        if let DeltaOp::Legacy(crpg_net::protocol::DeltaOp::Receipt {
                            seq,
                            processed_tick,
                            status,
                            ..
                        }) = op
                        {
                            self.receipts.push((*seq, *status, *processed_tick));
                        }
                    }
                    self.frames.push(frame);
                }
                Ok(None) => return,
                Err(reason) => panic!("player {} closed: {reason:?}", self.peer),
            }
        }
    }

    /// Reads until `cond` holds; a guard that panics `"deadline: {what}"`.
    fn read_until(&mut self, what: &str, cond: impl Fn(&Player) -> bool) {
        let start = Instant::now();
        loop {
            self.read();
            if cond(self) {
                return;
            }
            assert!(start.elapsed() < rig::DEADLINE, "deadline: {what}");
            self.quic.wait(Duration::from_millis(5));
        }
    }

    /// The replica built from every frame received.
    fn replica(&self) -> Replica {
        let mut replica = Replica::new();
        replica.apply(&self.frames);
        replica
    }

    /// `server_tick` of the newest frame.
    fn observed(&self) -> u64 {
        self.frames.last().expect("a frame").server_tick
    }

    /// Whether the replica shows `b` (replica id 2) dead.
    fn sees_b_dead(&self) -> bool {
        self.replica()
            .state
            .health
            .get(&2)
            .is_some_and(|(_, _, dead)| *dead)
    }

    /// Sends one strike declaration with the next seq.
    fn declare(&mut self, actor: u64, target: u64, observed: u64) {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.declared.insert(seq, (actor, target));
        let bytes = rig::declare(2, self.epoch, seq, observed, actor, rig::strike(), target);
        self.quic.try_send(&bytes).expect("client send");
    }

    /// B-30: handles new receipts (a `StaleTick` re-sends, anything else but
    /// `Applied` panics `"refused"`), then acts once on a new own turn.
    fn react(&mut self) {
        while self.handled < self.receipts.len() {
            let (seq, status, tick) = self.receipts[self.handled];
            self.handled += 1;
            match status {
                ReceiptStatus::Applied => {}
                ReceiptStatus::Rejected(RejectionCode::StaleTick) => {
                    self.stale_in_turn += 1;
                    self.stale_total += 1;
                    assert!(
                        self.stale_in_turn <= 3,
                        "refused: a fourth StaleTick for player {}",
                        self.peer
                    );
                    let (actor, target) = *self.declared.get(&seq).expect("a declared seq");
                    self.declare(actor, target, tick);
                }
                ReceiptStatus::Rejected(code) => {
                    panic!("refused: player {} seq {seq} {code:?}", self.peer)
                }
            }
        }
        if self.sees_b_dead() {
            return;
        }
        let Some(turn @ (Some(active), _)) = self.replica().state.turn else {
            return;
        };
        if !self.own.contains(&active) || self.acted == Some(turn) {
            return;
        }
        self.acted = Some(turn);
        self.stale_in_turn = 0;
        let observed = self.observed();
        self.declare(active, self.target, observed);
    }
}

/// B-28: waits until both players hold their first frame.
fn wait_first_frames(p1: &mut Player, p2: &mut Player) {
    p1.read_until("player 1's first frame", |p| !p.frames.is_empty());
    p2.read_until("player 2's first frame", |p| !p.frames.is_empty());
}

// ---------------------------------------------------------------------------
// B-29: the intrusion.
// ---------------------------------------------------------------------------

/// Checks one reject line of the binary: `t` is the loop time in
/// milliseconds (T040-E2: a decimal number below 30,000, not a multiple of
/// 50), then exactly `admission peer={peer} seq={seq} code={code} cached=false`.
fn assert_reject_line(line: &str, peer: u64, seq: u64, code: u8) {
    let rest = line
        .strip_prefix("crpg-server: reject t=")
        .unwrap_or_else(|| panic!("expected a reject line, got {line:?}"));
    let (t, rest) = rest.split_once(' ').expect("a time and a rejection");
    let t: u64 = t.parse().expect("t is a decimal number");
    assert!(t < 30_000, "t={t} is the loop time in milliseconds");
    assert_eq!(
        rest,
        format!("admission peer={peer} seq={seq} code={code} cached=false")
    );
}

/// B-29: player 2 ends the turn of `a`, which it does not control. Returns
/// the count of `StaleTick` re-sends the intrusion needed.
fn intrude(server: &mut Server, p2: &mut Player) -> u32 {
    let mut stale = 0;
    let mut observed = p2.observed();
    loop {
        let seq = p2.next_seq;
        p2.next_seq += 1;
        let bytes = rig::end_turn(2, p2.epoch, seq, observed, 1);
        p2.quic.try_send(&bytes).expect("client send");
        p2.read_until("the intrusion's receipt", |p| {
            p.receipts.iter().any(|(s, _, _)| *s == seq)
        });
        let (_, status, tick) = *p2
            .receipts
            .iter()
            .find(|(s, _, _)| *s == seq)
            .expect("the receipt");
        let line = server.next_line("the intrusion's reject line");
        match status {
            ReceiptStatus::Rejected(RejectionCode::NotAuthorized) => {
                assert_reject_line(&line, p2.peer, seq, 17);
                break;
            }
            ReceiptStatus::Rejected(RejectionCode::StaleTick) => {
                stale += 1;
                assert!(stale <= 3, "refused: a fourth StaleTick");
                assert_reject_line(&line, p2.peer, seq, 15);
                observed = tick;
            }
            other => panic!("refused: the intrusion's receipt is {other:?}, not code 17"),
        }
    }
    p2.handled = p2.receipts.len();
    stale
}

// ---------------------------------------------------------------------------
// B-30, B-31: the fight and its oracle.
// ---------------------------------------------------------------------------

/// B-30: both players react until both replicas show `b` dead.
fn fight(p1: &mut Player, p2: &mut Player) {
    let start = Instant::now();
    loop {
        p1.read();
        p2.read();
        p1.react();
        p2.react();
        if p1.sees_b_dead() && p2.sees_b_dead() {
            return;
        }
        assert!(start.elapsed() < rig::DEADLINE, "deadline: the fight");
        p1.quic.wait(Duration::from_millis(2));
        p2.quic.wait(Duration::from_millis(2));
    }
}

/// The six strikes in order: a to b, b to a, c to b, a to b, b to a, c to b.
fn strikes() -> Vec<CombatAction> {
    let strike = |actor, target| CombatAction::UseAbility {
        actor,
        ability: rig::strike(),
        target,
    };
    let (a, b, c) = (rig::a(), rig::b(), rig::c());
    vec![
        strike(a, b),
        strike(b, a),
        strike(c, b),
        strike(a, b),
        strike(b, a),
        strike(c, b),
    ]
}

/// B-31: `rig::fixture()` driven by the first `count` strikes, acknowledging
/// after each (`host_quic.rs`'s `mirror`).
fn mirror_after(count: usize) -> HistoryWorld {
    let mut oracle = rig::fixture();
    for action in strikes().into_iter().take(count) {
        oracle
            .perform_action(&action)
            .expect("the mirror accepts the same action");
        oracle
            .acknowledge(oracle.last_sequence())
            .expect("mirror acks");
    }
    oracle
}

/// The `(actor, target)` replica ids of every `ActionResolved`, in order.
fn resolved_pairs(replica: &Replica) -> Vec<(u64, u64)> {
    replica
        .events
        .iter()
        .filter_map(|op| match op {
            DeltaOp::ActionResolved { actor, target, .. } => Some((actor.get(), target.get())),
            _ => None,
        })
        .collect()
}

/// B-31: the pinned end state, the equality of both replicas and the mirror,
/// and the control that the mirror one strike short differs.
fn assert_end_state(p1: &Player, p2: &Player) {
    let pinned = BTreeMap::from([(1, (4, 10, false)), (2, (0, 10, true)), (3, (3, 3, false))]);
    let grants = rig::full_grants();
    let ids = rig::standard_ids();
    let expected = rig::expected_state(&mirror_after(6), &grants, &ids);
    assert_eq!(
        expected.health, pinned,
        "the mirror's healths are the pinned ones"
    );
    let (r1, r2) = (p1.replica(), p2.replica());
    for (name, state) in [("player 1", &r1.state), ("player 2", &r2.state)] {
        assert_eq!(state.health, pinned, "{name}'s healths");
        assert_eq!(state.entered, BTreeSet::from([1, 2, 3]), "{name}'s entered");
        assert_eq!(*state, expected, "{name} against the mirror");
    }
    assert_eq!(r1.state, r2.state, "the two replicas agree");
    let order = vec![(1, 2), (2, 1), (3, 2), (1, 2), (2, 1), (3, 2)];
    assert_eq!(resolved_pairs(&r1), order, "player 1's strikes");
    assert_eq!(resolved_pairs(&r2), order, "player 2's strikes");
    // Control: one strike short, the mirror's state is not the replicas'.
    let short: State = rig::expected_state(&mirror_after(5), &grants, &ids);
    assert_ne!(short, r1.state, "the comparison bites");
    assert_ne!(short, r2.state, "the comparison bites");
}

// ---------------------------------------------------------------------------
// B-32: the log of the whole run.
// ---------------------------------------------------------------------------

/// Every line is the start line, a reject line or the stop line; the rejects
/// are the intrusion's one code 17 and one code 15 per `StaleTick` re-send;
/// no line holds either credential's hex.
fn assert_lines(lines: &[String], stale: u32, saved: &Path) {
    assert_eq!(
        lines.last().map(String::as_str),
        Some(format!("crpg-server: stop reason=quit saved={}", saved.display()).as_str())
    );
    let rejects: Vec<&String> = lines
        .iter()
        .filter(|line| line.starts_with("crpg-server: reject "))
        .collect();
    let code17 = rejects.iter().filter(|l| l.contains(" code=17 ")).count();
    let code15 = rejects.iter().filter(|l| l.contains(" code=15 ")).count();
    assert_eq!(code17, 1, "one intrusion");
    assert_eq!(code15, stale as usize, "one code 15 per re-send");
    assert_eq!(rejects.len(), code17 + code15, "no other rejection");
    assert_eq!(lines.len(), rejects.len() + 2, "start, rejects, stop");
    for line in lines {
        assert!(
            !line.contains(&credential_hex(0x11)),
            "credential 1 in a line"
        );
        assert!(
            !line.contains(&credential_hex(0x22)),
            "credential 2 in a line"
        );
    }
}

// ---------------------------------------------------------------------------
// G1.
// ---------------------------------------------------------------------------

/// G1 (B-26 to B-32): the dedicated server, two real QUIC clients that fight
/// to identical visible state, a rejected message in the log, a clean quit
/// that saves. Control: the mirror one strike short differs from the replicas.
#[test]
fn dedicated_server_two_clients_fight_to_identical_state() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("t041d-g1");
    setup(&dir);
    // T041-E2: the quit must write the save, not leave setup's file in place.
    let setup_save = std::fs::read(dir.join("world.save")).expect("setup save");
    let mut server = Server::spawn(&dir);

    // B-28: player 1 first, player 2 after player 1's connect returned.
    let mut p1 = Player::connect(server.port, 0x11, 1, &[1, 3], 2);
    let mut p2 = Player::connect(server.port, 0x22, 2, &[2], 1);
    wait_first_frames(&mut p1, &mut p2);

    // B-29: one intrusion, logged as code 17 and not applied.
    let intrusion_stale = intrude(&mut server, &mut p2);

    // B-30, B-31: the fight and the comparison.
    fight(&mut p1, &mut p2);
    assert_end_state(&p1, &p2);

    // B-32: quit, exit 0, the save and the log.
    let stale = intrusion_stale + p1.stale_total + p2.stale_total;
    let (status, lines) = server.quit();
    assert_eq!(status.code(), Some(0), "a clean exit");
    let saved = dir.join("world.save");
    assert_lines(&lines, stale, &saved);
    let loaded = load_host_file(&saved, &id(), 9, 0);
    assert!(
        loaded.is_ok(),
        "the quit save loads: {:?}",
        loaded.as_ref().err()
    );
    let quit_save = std::fs::read(&saved).expect("quit save");
    assert_ne!(quit_save, setup_save, "the quit wrote the save (T041-E2)");
}
