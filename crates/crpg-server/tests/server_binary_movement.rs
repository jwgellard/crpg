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
use crpg_net::codec_v4;
use crpg_net::movement_v4::{encode_intent, MoveBody, MoveIntent};
use crpg_net::pool_v4::{
    GateMode, ReassemblyPoolV4, ReceiverIdV4, ReplicaUpdateV4, TransferLimitsV3,
};
use crpg_net::protocol::{NetId, LANE_COMBAT};
use crpg_net::protocol_transfer::{TransferId, TransferUp};
use crpg_net::protocol_v3::{
    DeltaOp as OpV3, PauseReason, PauseRights, ReceiptStatus, RejectionCode,
};
use crpg_net::protocol_v4::{DeltaOp, IntentBody, IntentFrame, SessionEpoch};
use crpg_net::replica_v3::PauseState;
use crpg_net::replica_v4::ReplicaWorldV4;
use crpg_net::transfer_v3;
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

// ---------------------------------------------------------------------------
// Fix round A: the new configuration's refusals and the invitation's mapping.
// ---------------------------------------------------------------------------

/// `movement_entry(credential_hex)` with `facts` and/or `rights` removed.
fn entry_without(credential_hex: &str, drop_facts: bool, drop_rights: bool) -> Value {
    let mut entry = movement_entry(credential_hex);
    let object = entry.as_object_mut().expect("object");
    if drop_facts {
        object.remove("facts");
    }
    if drop_rights {
        object.remove("rights");
    }
    entry
}

/// One refused run of the default (combat) configuration over `invitations`:
/// exit 1, exactly the one line `expected`, sidecar unchanged.
fn assert_combat_refused(name: &str, invitations: &Value, expected: &str) {
    let s = Setup::new(name);
    s.write_json("invitations.json", invitations);
    assert_refused(&s, expected);
}

/// B-M1: `"movement": false` and `"movement": null` are the combat wire
/// exactly: the combat start line, a `/3` hello refused, and a movement
/// entry refused as a combat world refuses it.
#[test]
fn movement_false_is_the_combat_wire() {
    for (name, value) in [("sm5_false", json!(false)), ("sm5_null", json!(null))] {
        // The start line and the endpoint are the default's.
        let s = Setup::new(name);
        let mut config = s.config();
        config["movement"] = value.clone();
        s.write_config(&config);
        let mut server = Server::spawn(&s.config_path(), true);
        let start = server.start();
        assert_eq!(start.line, start_line(start.port, 8, "none", "off"));
        assert!(matches!(
            connect_movement(start.port),
            Err(ConnectError::Handshake)
        ));
        assert!(hello(start.port, &x()).is_ok());
        drop(server);

        // A movement entry is refused as in a combat world.
        let s = Setup::new(&format!("{name}_entry"));
        let mut config = s.config();
        config["movement"] = value;
        s.write_config(&config);
        s.write_json("invitations.json", &json!([movement_entry(&hex(&x()))]));
        assert_refused(
            &s,
            "crpg-server: error invitations: entry 0: facts and rights need movement",
        );
    }
}

/// B-M2: in a combat world an entry with only `rights`, and one with only
/// `facts`, are each refused, naming their own index (entry 1, after a good
/// entry 0).
#[test]
fn combat_entry_with_one_of_facts_or_rights_is_refused() {
    let expected = "crpg-server: error invitations: entry 1: facts and rights need movement";
    let good = invitation_entry(&hex(&x()));

    // Only `rights`.
    let mut only_rights = invitation_entry(&hex(&y()));
    only_rights["rights"] = json!({"pause": false, "unpause": false});
    assert_combat_refused(
        "sm6_only_rights",
        &json!([good.clone(), only_rights]),
        expected,
    );

    // Only `facts`.
    let mut only_facts = invitation_entry(&hex(&y()));
    only_facts["facts"] = entry_without(&hex(&y()), false, true)["facts"].clone();
    assert_combat_refused("sm6_only_facts", &json!([good, only_facts]), expected);
}

/// Where in an entry a case adds the unknown key.
type Place = fn(&mut Value) -> &mut Value;

/// The unknown key every `deny_unknown_fields` case adds.
const UNKNOWN_KEY: &str = "bogus";

/// Each of the five new invitation types refuses an unknown key inside it:
/// exit 1, one line, serde's `unknown field` text under the `invitations`
/// prefix, sidecar unchanged.
#[test]
fn unknown_keys_in_the_new_invitation_types_are_refused() {
    let places: [(&str, Place); 5] = [
        ("facts", |e| &mut e["facts"]),
        ("entity", |e| &mut e["facts"]["entities"][0]),
        ("encounter", |e| &mut e["facts"]["encounter"]),
        ("host", |e| &mut e["facts"]["host"]),
        ("rights", |e| &mut e["rights"]),
    ];
    for (name, place) in places {
        let s = Setup::new(&format!("sm7_{name}"));
        let mut entry = movement_entry(&hex(&x()));
        place(&mut entry)[UNKNOWN_KEY] = json!(1);
        s.write_json("invitations.json", &json!([entry]));
        s.write_config(&movement_config(&s));
        let result = s.run_to_exit();
        let line = assert_one_error(&result);
        assert!(
            line.starts_with(&format!(
                "crpg-server: error invitations: unknown field `{UNKNOWN_KEY}`, expected "
            )),
            "{name}: {line}"
        );
        assert_eq!(s.sidecar(), "7\n");
    }
}
// ---------------------------------------------------------------------------
// The invitation's mapping, seen by a movement client (M9d-M9g, M10-M11).
// ---------------------------------------------------------------------------

/// The per-field flags of one movement invitation, in the key order of
/// `FactsEntry` and `RightsEntry`.
struct Flags {
    /// `[pools, effects, stats, tags]` of entities `a`, `b` and `c`.
    entities: [[bool; 4]; 3],
    /// `[mode, pending_mode, clock, timeline]`.
    encounter: [bool; 4],
    /// `[pause_reason, auto_pause, turn_timer]`.
    host: [bool; 3],
    /// `[pause, unpause]`.
    rights: [bool; 2],
    /// The indices (0 = `a`) the invitation controls.
    control: Vec<u32>,
}

/// A movement entry for `X` carrying exactly `flags`.
fn flagged_entry(flags: &Flags) -> Value {
    let entity = |index: u32| json!({"index": index, "generation": 1});
    let mut entry = movement_entry(&hex(&x()));
    entry["control"] = json!(flags.control.iter().map(|i| entity(*i)).collect::<Vec<_>>());
    let rows: Vec<Value> = flags
        .entities
        .iter()
        .enumerate()
        .map(|(i, f)| {
            json!({
                "entity": entity(i as u32),
                "pools": f[0], "effects": f[1], "stats": f[2], "tags": f[3]
            })
        })
        .collect();
    let e = flags.encounter;
    let h = flags.host;
    entry["facts"] = json!({
        "entities": rows,
        "encounter": {"mode": e[0], "pending_mode": e[1], "clock": e[2], "timeline": e[3]},
        "host": {"pause_reason": h[0], "auto_pause": h[1], "turn_timer": h[2]}
    });
    entry["rights"] = json!({"pause": flags.rights[0], "unpause": flags.rights[1]});
    entry
}

/// A movement client that keeps a v4 replica of what the server sends and
/// records every lane-0 receipt (the reading half of
/// `tests/host_lane1.rs`'s `Client`, over a real connection).
struct Viewer {
    client: QuicClient,
    epoch: [u8; 16],
    pool: ReassemblyPoolV4,
    rid: ReceiverIdV4,
    replica: ReplicaWorldV4,
    snapshot: Option<TransferId>,
    receipts: Vec<(u64, ReceiptStatus)>,
    started: Instant,
}

impl Viewer {
    fn connect(port: u16) -> Viewer {
        let client = connect_movement(port).expect("a movement hello connects");
        let epoch = client.welcome().epoch;
        let mut pool = ReassemblyPoolV4::new(TransferLimitsV3::v3()).expect("v3 limits");
        let rid = pool
            .add_receiver(epoch, GateMode::AwaitSnapshot)
            .expect("receiver");
        Viewer {
            client,
            epoch,
            pool,
            rid,
            replica: ReplicaWorldV4::new(SessionEpoch(epoch)),
            snapshot: None,
            receipts: Vec::new(),
            started: Instant::now(),
        }
    }

    /// Reads everything available, feeds the gate and applies what it
    /// releases; publishes the first snapshot.
    fn pump(&mut self) {
        let now = self.started.elapsed().as_millis() as u64;
        loop {
            match self.client.try_recv_channel() {
                Ok(Some((Channel::Resume, _))) => {}
                Ok(Some((Channel::Transfer, bytes))) => {
                    self.pool
                        .offer_transfer(self.rid, &bytes, now)
                        .expect("the gate accepts the transfer frame");
                }
                Ok(Some((Channel::Lane0, bytes))) => {
                    self.pool
                        .offer_delta(self.rid, &bytes, now)
                        .expect("the gate accepts the delta frame");
                }
                Ok(None) => break,
                Err(reason) => panic!("the connection closed: {reason:?}"),
            }
        }
        while let Some(update) = self.pool.pop_update(self.rid) {
            match update {
                ReplicaUpdateV4::Snapshot(publication) => {
                    self.snapshot = Some(publication.transfer);
                    self.replica.apply_snapshot(&publication).expect("applies");
                    let bytes = transfer_v3::encode_up(&TransferUp::Published {
                        epoch: self.epoch,
                        transfer: publication.transfer,
                    })
                    .expect("encodes");
                    self.client
                        .try_send_channel(Channel::Transfer, &bytes)
                        .expect("publishes");
                }
                ReplicaUpdateV4::Delta(frame) => {
                    for op in &frame.ops {
                        if let DeltaOp::V3(OpV3::Receipt { seq, status, .. }) = op {
                            self.receipts.push((*seq, *status));
                        }
                    }
                    self.replica.apply_delta(&frame).expect("applies");
                }
            }
        }
    }

    /// Guard: pumps until `cond` holds, panicking `"deadline"` after
    /// `rig::DEADLINE`.
    fn until(&mut self, what: &str, cond: impl Fn(&Viewer) -> bool) {
        let start = Instant::now();
        loop {
            self.pump();
            if cond(self) {
                return;
            }
            assert!(start.elapsed() < rig::DEADLINE, "deadline: {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Waits for the first anchored image: a snapshot and the host facts.
    fn ready(&mut self) {
        self.until("the first anchored image", |v| {
            v.snapshot.is_some() && v.replica.v3().host_facts().is_some()
        });
    }

    /// Sends one lane-0 intent for the actor `n(actor)`.
    fn send(&mut self, seq: u64, actor: u64, body: IntentBody) {
        let bytes = codec_v4::encode_intent(&IntentFrame {
            epoch: self.epoch,
            lane: LANE_COMBAT,
            seq,
            observed_tick: self.replica.v3().server_tick(),
            actor: NetId::new(actor).expect("nonzero"),
            body,
        })
        .expect("encodes");
        self.client
            .try_send_channel(Channel::Lane0, &bytes)
            .expect("sends on lane 0");
    }

    /// Guard: the receipt of `seq`.
    fn receipt(&mut self, seq: u64) -> ReceiptStatus {
        self.until("a receipt", |v| v.receipts.iter().any(|(s, _)| *s == seq));
        self.receipts
            .iter()
            .find(|(s, _)| *s == seq)
            .map(|(_, status)| *status)
            .expect("the receipt is there")
    }
}

/// What the first image must show of `flags`: each family is disclosed
/// exactly when its flag is true (`facts_v3.rs`: `None` is withheld), the
/// standing order follows control, and the host facts carry the rights
/// whatever the grants say.
fn assert_image_follows(viewer: &Viewer, flags: &Flags) {
    let replica = viewer.replica.v3();
    let rows = replica.entities();
    assert_eq!(rows.len(), 3);
    for (i, row) in rows.iter().enumerate() {
        let f = flags.entities[i];
        let controlled = flags.control.contains(&(i as u32));
        assert_eq!(row.net_id, NetId::new(i as u64 + 1).expect("nonzero"));
        assert_eq!(
            [
                row.facts.pools.is_some(),
                row.facts.effects.is_some(),
                row.facts.stats.is_some(),
                row.facts.tags.is_some(),
                row.facts.order.is_some(),
            ],
            [f[0], f[1], f[2], f[3], controlled],
            "entity {i}"
        );
    }
    let encounter = replica.encounter();
    assert_eq!(
        [
            encounter.mode.is_some(),
            encounter.pending_mode.is_some(),
            encounter.clock.is_some(),
            replica.timeline().is_some(),
        ],
        flags.encounter
    );
    let host = replica.host_facts().expect("host facts");
    assert_eq!(
        host.rights,
        PauseRights {
            pause: flags.rights[0],
            unpause: flags.rights[1],
        }
    );
    assert_eq!(host.auto_pause.is_some(), flags.host[1]);
    assert_eq!(host.turn_timer_ms.is_some(), flags.host[2]);
}

/// The viewer's pause state as a plain value.
fn assert_pause(viewer: &Viewer, expected: PauseState) {
    assert_eq!(viewer.replica.v3().pause(), Some(expected));
}

/// Runs the binary over `flags` and checks, as the client of the invitation,
/// the first image, then a `Pause` by actor `n(actor)` and an `Unpause`.
fn run_flagged(name: &str, flags: &Flags, actor: u64) {
    let s = Setup::new(name);
    s.write_json("invitations.json", &json!([flagged_entry(flags)]));
    s.write_config(&movement_config(&s));
    let mut server = Server::spawn(&s.config_path(), true);
    let start = server.start();
    let mut viewer = Viewer::connect(start.port);
    viewer.ready();
    assert_image_follows(&viewer, flags);

    // `Pause` needs the `pause` right; its reason is disclosed exactly when
    // `host.pause_reason` is (`PauseReason::Player`).
    let [pause, unpause] = flags.rights;
    let paused = PauseState::Paused {
        reason: flags.host[0].then_some(PauseReason::Player),
    };
    viewer.send(1, actor, IntentBody::Pause);
    if pause {
        assert_eq!(viewer.receipt(1), ReceiptStatus::Applied);
        viewer.until("the pause", |v| v.replica.v3().pause() == Some(paused));
        assert_pause(&viewer, paused);
    } else {
        assert_eq!(
            viewer.receipt(1),
            ReceiptStatus::Rejected(RejectionCode::NoPauseRight)
        );
        assert_pause(&viewer, PauseState::Running);
    }

    // `Unpause` needs the `unpause` right (an unpause of a running host is
    // applied and changes nothing).
    viewer.send(2, actor, IntentBody::Unpause);
    if unpause {
        assert_eq!(viewer.receipt(2), ReceiptStatus::Applied);
        viewer.until("the unpause", |v| {
            v.replica.v3().pause() == Some(PauseState::Running)
        });
        assert_pause(&viewer, PauseState::Running);
    } else {
        assert_eq!(
            viewer.receipt(2),
            ReceiptStatus::Rejected(RejectionCode::NoPauseRight)
        );
        // Still as the pause left it.
        assert_pause(&viewer, if pause { paused } else { PauseState::Running });
    }

    server.send("quit\n");
    assert_eq!(server.wait().code(), Some(0));
    assert_no_credential(&server.rest());
}

/// B-M4: a movement invitation's `control`, `grants`, `facts` and `rights`
/// reach the host field for field. Entity flags differ within and across
/// entities, the encounter flags alternate, the host flags and the rights
/// differ, and the viewer reads the first image and tries a pause and an
/// unpause. This run may pause, may not unpause, and controls `a` and `c`.
#[test]
fn invitation_mapping_pause_allowed_unpause_denied() {
    run_flagged(
        "sm8_pause_only",
        &Flags {
            entities: [
                [true, false, true, false],
                [false, true, false, true],
                [true, true, false, false],
            ],
            encounter: [true, false, true, false],
            host: [true, false, false],
            rights: [true, false],
            control: vec![0, 2],
        },
        1,
    );
}

/// B-M4, the other polarity: this run may not pause, may unpause, discloses
/// the other host flags, and controls `b` alone.
#[test]
fn invitation_mapping_pause_denied_unpause_allowed() {
    run_flagged(
        "sm8_unpause_only",
        &Flags {
            entities: [
                [false, true, false, true],
                [true, false, true, false],
                [false, false, true, true],
            ],
            encounter: [false, true, false, true],
            host: [false, true, false],
            rights: [false, true],
            control: vec![1],
        },
        2,
    );
}
