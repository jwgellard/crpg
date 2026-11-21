//! T155d acceptance suite (`tasks/T155.md` B-M1..B-M6, SM1..SM4): the dedicated
//! binary with `"movement": true`, black box.
//!
//! As `tests/server_binary.rs` (T040b), whose `Setup`, `Server`, `hex`, `x()`
//! and `start_line` are copied below: every case spawns the built
//! `crpg-server` over a fresh directory of files made from the shared rig's
//! fixture host, and asserts on its exit code, standard error, files and the
//! QUIC clients it answers. The movement client calls are those of
//! `tests/host_quic_move_resume.rs` and `tests/host_quic_move.rs`. Every wait
//! is a guard that panics `"deadline"` after `rig::DEADLINE` of real time,
//! never an oracle.

#[path = "support/quic_host_rig.rs"]
mod rig;

use std::io::{BufRead, BufReader, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crpg_core::Ulid;
use crpg_net::movement_v4::{encode_intent, MoveBody, MoveIntent};
use crpg_net::protocol::NetId;
use crpg_net_quic::{
    Channel, ClientLimits, ConnectError, Framing, Hello, QuicClient, MOVEMENT_WIRE_VERSION,
};
use crpg_server::save::{load_host_file, save_host_file, SaveIdentity};
use serde_json::{json, Value};

const BIN: &str = env!("CARGO_BIN_EXE_crpg-server");

/// `crpg_core::Ulid::from_u128(n)`, the text of the contract's `u(n)`.
fn u(n: u128) -> Ulid {
    Ulid::from_u128(n)
}

/// `ID`: the campaign identity of the set-up's save.
fn id() -> SaveIdentity {
    SaveIdentity {
        campaign_id: u(1),
        campaign_version: "1.0.0".to_owned(),
    }
}

/// The credential `X`: 32 bytes of `0x11`.
fn x() -> Vec<u8> {
    vec![0x11; 32]
}

/// The credential `Y`: 32 bytes of `0x22`.
fn y() -> Vec<u8> {
    vec![0x22; 32]
}

/// `hex(bytes)`: lowercase hex.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ---------------------------------------------------------------------------
// Set-up: the directory of files every case runs over.
// ---------------------------------------------------------------------------

/// One case's directory, set up as the contract's `setup(dir)`.
struct Setup {
    dir: PathBuf,
}

/// One invitation entry of the invitations file with full grants for a, b, c.
fn invitation_entry(credential_hex: &str) -> Value {
    let flags = |e: Value| {
        json!({
            "entity": e, "present": true, "health": true, "spawn": true, "despawn": true,
            "died": true, "action_actor": true, "action_target": true,
            "action_outcome": true, "action_damage": true, "turn_start": true
        })
    };
    let entity = |index: u32| json!({"index": index, "generation": 1});
    json!({
        "credential": credential_hex,
        "control": [entity(0), entity(1), entity(2)],
        "grants": {
            "entities": [flags(entity(0)), flags(entity(1)), flags(entity(2))],
            "abilities": [rig::strike(), rig::smite(), rig::whiff()],
            "encounters": [rig::encounter()],
            "turn_round": true,
            "encounter_end": true,
            "turn_state": true
        }
    })
}

impl Setup {
    /// A fresh directory named `name` with the certificate, key, save,
    /// sidecar (`7\n`), invitations (one entry, `hex(X)`) and config.
    fn new(name: &str) -> Setup {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join("server_binary_movement")
            .join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("creates the case directory");
        let setup = Setup { dir };
        std::fs::write(setup.path("server.cert.der"), rig::SERVER_A_CERT).expect("cert");
        std::fs::write(setup.path("server.key.pk8.der"), rig::SERVER_A_KEY).expect("key");
        save_host_file(&setup.path("world.save"), &rig::new_host(rig::V2), &id())
            .expect("saves the fixture host");
        std::fs::write(setup.path("world.save.incarnation"), "7\n").expect("sidecar");
        setup.write_json("invitations.json", &json!([invitation_entry(&hex(&x()))]));
        setup.write_config(&setup.config());
        setup
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// The configuration file's path, as the process is given it.
    fn config_path(&self) -> PathBuf {
        self.path("config.json")
    }

    /// The default configuration.
    fn config(&self) -> Value {
        json!({
            "listen": "127.0.0.1:0",
            "certificate": "server.cert.der",
            "private_key": "server.key.pk8.der",
            "invitations": "invitations.json",
            "save": "world.save",
            "campaign_id": u(1),
            "campaign_version": "1.0.0"
        })
    }

    fn write_json(&self, name: &str, value: &Value) {
        std::fs::write(self.path(name), serde_json::to_vec(value).expect("json")).expect("write");
    }

    fn write_config(&self, config: &Value) {
        self.write_json("config.json", config);
    }

    /// The sidecar's text.
    fn sidecar(&self) -> String {
        std::fs::read_to_string(self.path("world.save.incarnation")).expect("sidecar reads")
    }

    /// Runs the binary to its end with no standard input; returns the exit
    /// code and every standard error line.
    fn run_to_exit(&self) -> (i32, Vec<String>) {
        let mut server = Server::spawn(&self.config_path(), false);
        let status = server.wait();
        let lines = server.rest();
        (status.code().expect("exits with a code"), lines)
    }

    /// Whether the saved world loads at `incarnation` (time 0).
    fn loads_at(&self, incarnation: u64) -> bool {
        load_host_file(&self.path("world.save"), &id(), incarnation, 0).is_ok()
    }
}

// ---------------------------------------------------------------------------
// The process under test.
// ---------------------------------------------------------------------------

/// The fields of the start line.
struct Start {
    line: String,
    port: u16,
}

/// A spawned server with its standard error read on a helper thread.
struct Server {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
    reader: Option<JoinHandle<()>>,
    seen: Vec<String>,
}

impl Server {
    /// Spawns the binary over `config`. `open_stdin`: standard input is a pipe
    /// the case keeps open; otherwise it is closed at spawn.
    fn spawn(config: &Path, open_stdin: bool) -> Server {
        let mut child = Command::new(BIN)
            .arg("--config")
            .arg(config)
            .stdin(if open_stdin {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawns the server");
        let stdin = child.stdin.take();
        let stderr = child.stderr.take().expect("piped stderr");
        let (send, lines) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            let mut reader = BufReader::new(stderr);
            loop {
                let mut buf = Vec::new();
                match reader.read_until(b'\n', &mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        let text = String::from_utf8_lossy(&buf).into_owned();
                        assert!(text.ends_with('\n'), "every line ends with a newline");
                        let _ = send.send(text[..text.len() - 1].to_owned());
                    }
                }
            }
        });
        Server {
            child,
            stdin,
            lines,
            reader: Some(reader),
            seen: Vec::new(),
        }
    }

    /// The next standard error line.
    fn next_line(&mut self) -> String {
        match self.lines.recv_timeout(rig::DEADLINE) {
            Ok(line) => {
                self.seen.push(line.clone());
                line
            }
            Err(RecvTimeoutError::Timeout) => panic!("deadline: a standard error line"),
            Err(RecvTimeoutError::Disconnected) => {
                panic!("standard error closed; lines so far: {:?}", self.seen)
            }
        }
    }

    /// Reads the start line and parses its port.
    fn start(&mut self) -> Start {
        let line = self.next_line();
        let rest = line
            .strip_prefix("crpg-server: start listen=127.0.0.1:")
            .unwrap_or_else(|| panic!("expected the start line, got {line:?}"));
        let port = rest
            .split(' ')
            .next()
            .and_then(|p| p.parse().ok())
            .unwrap_or_else(|| panic!("no port in {line:?}"));
        Start { line, port }
    }

    /// Writes text to standard input.
    fn send(&mut self, text: &str) {
        let stdin = self.stdin.as_mut().expect("standard input is open");
        stdin.write_all(text.as_bytes()).expect("writes stdin");
        stdin.flush().expect("flushes stdin");
    }

    /// Waits for the process to end, at most `rig::DEADLINE`.
    fn wait(&mut self) -> ExitStatus {
        let start = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().expect("try_wait") {
                return status;
            }
            assert!(start.elapsed() < rig::DEADLINE, "deadline: the exit");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Every remaining standard error line, once the process has ended.
    fn rest(&mut self) -> Vec<String> {
        while let Ok(line) = self.lines.recv_timeout(rig::DEADLINE) {
            self.seen.push(line);
        }
        if let Some(reader) = self.reader.take() {
            reader.join().expect("reader ends");
        }
        self.seen.clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A `/2` hello with `credential` to the server on `port`.
fn hello(port: u16, credential: &[u8]) -> Result<QuicClient, ConnectError> {
    let server = SocketAddr::from(([127, 0, 0, 1], port));
    let hello = Hello {
        wire_version: rig::version_byte(rig::V2),
        credential: rig::credential(credential),
    };
    QuicClient::connect_framed(
        rig::client_config(ClientLimits::v1()),
        server,
        &hello,
        Framing::V2,
    )
}

/// The expected start line for a default set-up.
fn start_line(port: u16, incarnation: u64, nav: &str, ai: &str) -> String {
    format!(
        "crpg-server: start listen=127.0.0.1:{port} incarnation={incarnation} pin={} nav={nav} ai={ai} invitations=1",
        rig::SERVER_A_PIN_HEX
    )
}

/// No line holds either credential's hex.
fn assert_no_credential(lines: &[String]) {
    for line in lines {
        assert!(!line.contains(&hex(&x())), "credential X in {line:?}");
        assert!(!line.contains(&hex(&y())), "credential Y in {line:?}");
    }
}
// ---------------------------------------------------------------------------
// The movement set-up (T155d).
// ---------------------------------------------------------------------------

/// `invitation_entry(hex)` for a movement world: control over `a` alone, all
/// three entities' fact flags, every encounter and host flag, and no pause
/// right.
fn movement_entry(credential_hex: &str) -> Value {
    let entity = |index: u32| json!({"index": index, "generation": 1});
    let flags = |index: u32| {
        json!({
            "entity": entity(index),
            "pools": true, "effects": true, "stats": true, "tags": true
        })
    };
    let mut entry = invitation_entry(credential_hex);
    entry["control"] = json!([entity(0)]);
    entry["facts"] = json!({
        "entities": [flags(0), flags(1), flags(2)],
        "encounter": {"mode": true, "pending_mode": true, "clock": true, "timeline": true},
        "host": {"pause_reason": true, "auto_pause": true, "turn_timer": true}
    });
    entry["rights"] = json!({"pause": false, "unpause": false});
    entry
}

/// The default configuration plus `"movement": true` and an area.
fn movement_config(setup: &Setup) -> Value {
    let mut config = setup.config();
    config["movement"] = json!(true);
    config["area"] = json!({"min": [0, 0, 0], "max": [16, 0, 16]});
    config
}

/// A set-up whose invitations file holds one movement entry for `X` and whose
/// configuration is [`movement_config`].
fn movement_setup(name: &str) -> Setup {
    let setup = Setup::new(name);
    setup.write_json("invitations.json", &json!([movement_entry(&hex(&x()))]));
    setup.write_config(&movement_config(&setup));
    setup
}

/// A movement (`crpg-lane0/3`) client of `X` on `port`.
fn connect_movement(port: u16) -> Result<QuicClient, ConnectError> {
    let server = SocketAddr::from(([127, 0, 0, 1], port));
    let hello = Hello {
        wire_version: MOVEMENT_WIRE_VERSION,
        credential: rig::credential(&x()),
    };
    QuicClient::connect_movement(rig::client_config(ClientLimits::v1()), server, &hello)
}

/// The canonical bytes of a `Halt` intent.
fn halt_bytes(epoch: [u8; 16], seq: u64, actor: u64) -> Vec<u8> {
    encode_intent(&MoveIntent {
        epoch,
        seq,
        actor: NetId::new(actor).expect("nonzero"),
        body: MoveBody::Halt,
    })
    .expect("encodes")
}

/// The first lane-stream frame the client reads, guarded by `rig::DEADLINE`.
fn first_frame(client: &mut QuicClient) -> (Channel, Vec<u8>) {
    let start = Instant::now();
    loop {
        match client.try_recv_channel() {
            Ok(Some(entry)) => return entry,
            Ok(None) => {}
            Err(reason) => panic!("the connection closed first: {reason:?}"),
        }
        assert!(start.elapsed() < rig::DEADLINE, "deadline: the first frame");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Asserts one failed run: exit 1, one line starting `crpg-server: error `.
fn assert_one_error(result: &(i32, Vec<String>)) -> &str {
    assert_eq!(result.0, 1, "{:?}", result.1);
    assert_eq!(result.1.len(), 1, "{:?}", result.1);
    assert!(
        result.1[0].starts_with("crpg-server: error "),
        "{:?}",
        result.1
    );
    &result.1[0]
}

/// One refused run: exit 1, exactly the one line `expected`, sidecar `7\n`.
fn assert_refused(setup: &Setup, expected: &str) {
    let result = setup.run_to_exit();
    assert_eq!(assert_one_error(&result), expected);
    assert_eq!(setup.sidecar(), "7\n");
}

// ---------------------------------------------------------------------------
// SM1 `movement_config_errors_spend_nothing` (B-M1, B-M2).
// ---------------------------------------------------------------------------

#[test]
fn movement_config_errors_spend_nothing() {
    // B-M1: movement without an area.
    let s = Setup::new("sm1_no_area");
    let mut config = movement_config(&s);
    config.as_object_mut().expect("object").remove("area");
    s.write_config(&config);
    assert_refused(&s, "crpg-server: error config: movement needs area");

    // B-M2: a movement entry without rights.
    let s = movement_setup("sm1_no_rights");
    let mut entry = movement_entry(&hex(&x()));
    entry.as_object_mut().expect("object").remove("rights");
    s.write_json("invitations.json", &json!([entry]));
    assert_refused(
        &s,
        "crpg-server: error invitations: entry 0: movement needs facts and rights",
    );

    // B-M2: a movement entry without facts, as the second entry.
    let s = movement_setup("sm1_no_facts");
    let mut entry = movement_entry(&hex(&y()));
    entry.as_object_mut().expect("object").remove("facts");
    s.write_json(
        "invitations.json",
        &json!([movement_entry(&hex(&x())), entry]),
    );
    assert_refused(
        &s,
        "crpg-server: error invitations: entry 1: movement needs facts and rights",
    );

    // B-M2: the default (combat) config with a movement entry.
    let s = Setup::new("sm1_combat_with_facts");
    s.write_json("invitations.json", &json!([movement_entry(&hex(&x()))]));
    assert_refused(
        &s,
        "crpg-server: error invitations: entry 0: facts and rights need movement",
    );

    // A `movement` that is not a boolean is a configuration error.
    let s = Setup::new("sm1_not_a_bool");
    let mut config = movement_config(&s);
    config["movement"] = json!("yes");
    s.write_config(&config);
    let result = s.run_to_exit();
    let line = assert_one_error(&result);
    assert!(line.starts_with("crpg-server: error config: "), "{line}");
    assert_eq!(s.sidecar(), "7\n");
}

// ---------------------------------------------------------------------------
// SM2 `movement_endpoint_serves_v4_and_issues_a_ticket` (B-M3..B-M5).
// ---------------------------------------------------------------------------

#[test]
fn movement_endpoint_serves_v4_and_issues_a_ticket() {
    let s = movement_setup("sm2");
    let mut server = Server::spawn(&s.config_path(), true);
    let start = server.start();
    assert_eq!(
        start.line,
        start_line(start.port, 8, "bound", "off") + " movement=on"
    );

    let mut client = connect_movement(start.port).expect("a movement hello connects");
    let welcome = client.welcome();
    assert_eq!(welcome.wire_version, 4);
    assert_eq!(welcome.epoch, rig::epoch_of(8, 1));

    // The first lane-stream frame is the resume ticket, byte for byte.
    let mut message = b"crpg-resume-proof/1".to_vec();
    message.extend_from_slice(&welcome.epoch);
    let mut ticket = vec![0x91, 0x03, 0x30];
    ticket.extend_from_slice(&welcome.epoch);
    ticket.push(0x20);
    ticket.extend_from_slice(blake3::keyed_hash(&[0x11; 32], &message).as_bytes());
    assert_eq!(first_frame(&mut client), (Channel::Resume, ticket));

    // A `/2` hello does not reach the movement endpoint.
    assert!(matches!(
        hello(start.port, &x()),
        Err(ConnectError::Handshake)
    ));

    server.send("quit\n");
    assert_eq!(server.wait().code(), Some(0));
    let lines = server.rest();
    assert_eq!(
        lines.last().expect("a last line"),
        &format!(
            "crpg-server: stop reason=quit saved={}",
            s.path("world.save").display()
        )
    );
    assert_no_credential(&lines);
    assert!(s.loads_at(9));
    assert!(!s.loads_at(8));
}

// ---------------------------------------------------------------------------
// SM3 `the_combat_endpoint_is_unchanged` (B-M3, B-M5).
// ---------------------------------------------------------------------------

#[test]
fn the_combat_endpoint_is_unchanged() {
    let s = Setup::new("sm3");
    let mut server = Server::spawn(&s.config_path(), true);
    let start = server.start();
    assert_eq!(start.line, start_line(start.port, 8, "none", "off"));
    assert!(matches!(
        connect_movement(start.port),
        Err(ConnectError::Handshake)
    ));
    assert!(hello(start.port, &x()).is_ok());
}

// ---------------------------------------------------------------------------
// SM4 `a_refused_move_is_a_reject_line` (B-M6, T155c C-5).
// ---------------------------------------------------------------------------

#[test]
fn a_refused_move_is_a_reject_line() {
    let s = movement_setup("sm4");
    let mut server = Server::spawn(&s.config_path(), true);
    let start = server.start();
    let mut client = connect_movement(start.port).expect("a movement hello connects");

    // Actor 2 is not in `X`'s control (`[a]`): refused at admission.
    let frame = halt_bytes(client.welcome().epoch, 1, 2);
    client.try_send_lane1(&frame).expect("sends on lane 1");

    let line = server.next_line();
    let rest = line
        .strip_prefix("crpg-server: reject t=")
        .unwrap_or_else(|| panic!("expected a reject line, got {line:?}"));
    let (t, rest) = rest.split_once(' ').expect("a time and a rejection");
    assert!(
        !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()),
        "{line:?}"
    );
    let rest = rest.strip_prefix("move conn=").expect("a move rejection");
    let (conn, rest) = rest.split_once(' ').expect("a connection and the rest");
    assert!(
        !conn.is_empty() && conn.bytes().all(|b| b.is_ascii_digit()),
        "{line:?}"
    );
    assert_eq!(
        rest,
        format!("peer=1 frame=1 len={} seq=1 code=17", frame.len())
    );

    server.send("quit\n");
    assert_eq!(server.wait().code(), Some(0));
    assert_no_credential(&server.rest());
}
