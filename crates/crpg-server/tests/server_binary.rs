//! T040b acceptance suite (`tasks/T040.md` S1-S11, B-10 to B-19): the
//! dedicated binary, black box.
//!
//! Every case spawns the built `crpg-server` (`CARGO_BIN_EXE_crpg-server`)
//! over a fresh directory of files made from the shared rig's fixture host,
//! and asserts on what the process does: its exit code, its standard output
//! and error, the files it leaves, and the QUIC clients it answers. The rig
//! is `tests/support/quic_host_rig.rs`. Every wait is a guard that panics
//! `"deadline"` after `rig::DEADLINE` of real time, never an oracle.

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
use crpg_net_quic::{ClientLimits, ConnectError, Framing, Hello, QuicClient};
use crpg_server::save::{load_host_file, save_host_file, SaveIdentity};
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use serde_json::{json, Value};

const BIN: &str = env!("CARGO_BIN_EXE_crpg-server");

/// The two usage lines of `--help`.
const USAGE: &str = "Usage: crpg-server --config <path>\n       crpg-server --help | --version\n";

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
            .join("server_binary")
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

    /// Kills the process and reaps it.
    fn kill(&mut self) {
        self.child.kill().expect("kills the server");
        let _ = self.child.wait();
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
// S1, S2.
// ---------------------------------------------------------------------------

fn output(args: &[&str]) -> std::process::Output {
    Command::new(BIN)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("runs the binary")
}

/// S1 (B-10): help and version print to standard output and exit 0; `--help`
/// beats everything else. Control: `--bogus --help` is help.
#[test]
fn help_and_version() {
    let help = output(&["--help"]);
    assert_eq!(help.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&help.stdout), USAGE);
    assert!(help.stderr.is_empty(), "{:?}", help.stderr);

    let version = output(&["--version"]);
    assert_eq!(version.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&version.stdout),
        format!("crpg-server {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert!(version.stderr.is_empty(), "{:?}", version.stderr);

    // Control: help outranks an unknown argument, and `--help` outranks
    // `--version`.
    let control = output(&["--bogus", "--help"]);
    assert_eq!(control.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&control.stdout), USAGE);
    assert!(control.stderr.is_empty());
    let both = output(&["--version", "--help"]);
    assert_eq!(String::from_utf8_lossy(&both.stdout), USAGE);
    let version_first = output(&["--bogus", "--version"]);
    assert_eq!(version_first.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&version_first.stdout),
        format!("crpg-server {}\n", env!("CARGO_PKG_VERSION"))
    );
}

/// S2 (B-10): usage errors are one line on standard error and exit 2.
#[test]
fn usage_errors() {
    let cases: &[(&[&str], &str)] = &[
        (&[], "crpg-server: usage: missing --config\n"),
        (
            &["--config"],
            "crpg-server: usage: --config needs a value\n",
        ),
        (
            &["--config", "c.json", "x"],
            "crpg-server: usage: unknown argument x\n",
        ),
        (&["x"], "crpg-server: usage: unknown argument x\n"),
        (
            &["x", "--config", "c.json"],
            "crpg-server: usage: unknown argument x\n",
        ),
    ];
    for (args, expected) in cases {
        let out = output(args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert_eq!(String::from_utf8_lossy(&out.stderr), *expected, "{args:?}");
        assert!(out.stdout.is_empty(), "{args:?}: {:?}", out.stdout);
    }
}

// ---------------------------------------------------------------------------
// S3, S4.
// ---------------------------------------------------------------------------

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

/// S3 (B-11 stages 1-5): a configuration, invitation or area refusal exits 1
/// with one line and spends no incarnation.
#[test]
fn config_errors_spend_nothing() {
    // A missing config path.
    let s = Setup::new("s3_missing");
    let mut server = Server::spawn(&s.path("nope.json"), false);
    assert_eq!(server.wait().code(), Some(1));
    let lines = server.rest();
    assert_one_error(&(1, lines.clone()));
    assert!(
        lines[0].starts_with("crpg-server: error config: "),
        "{lines:?}"
    );
    assert_eq!(s.sidecar(), "7\n");

    // An unknown key.
    let s = Setup::new("s3_unknown_key");
    let mut config = s.config();
    config["port"] = json!(1);
    s.write_config(&config);
    let result = s.run_to_exit();
    let line = assert_one_error(&result);
    assert!(line.starts_with("crpg-server: error config: "), "{line}");
    assert_eq!(s.sidecar(), "7\n");

    // An area value out of range.
    let s = Setup::new("s3_area");
    let mut config = s.config();
    config["area"] = json!({"min": [40000, 0, 0], "max": [64, 0, 64]});
    s.write_config(&config);
    let result = s.run_to_exit();
    assert_eq!(
        assert_one_error(&result),
        "crpg-server: error config: area value out of range"
    );
    assert_eq!(s.sidecar(), "7\n");

    // Likewise a `max` value out of range.
    let s = Setup::new("s3_area_max");
    let mut config = s.config();
    config["area"] = json!({"min": [0, 0, 0], "max": [64, 0, -32769]});
    s.write_config(&config);
    let result = s.run_to_exit();
    assert_eq!(
        assert_one_error(&result),
        "crpg-server: error config: area value out of range"
    );
    assert_eq!(s.sidecar(), "7\n");

    // No invitations.
    let s = Setup::new("s3_no_invitations");
    s.write_json("invitations.json", &json!([]));
    let result = s.run_to_exit();
    assert_eq!(
        assert_one_error(&result),
        "crpg-server: error invitations: no invitations"
    );
    assert_eq!(s.sidecar(), "7\n");

    // Too many invitations.
    let s = Setup::new("s3_too_many");
    let entries: Vec<Value> = (0..65u8)
        .map(|i| invitation_entry(&hex(&[i; 32])))
        .collect();
    s.write_json("invitations.json", &Value::Array(entries));
    let result = s.run_to_exit();
    assert_eq!(
        assert_one_error(&result),
        "crpg-server: error invitations: too many invitations"
    );
    assert_eq!(s.sidecar(), "7\n");

    // A 63-character credential, and a non-lowercase one.
    for (name, bad) in [
        ("s3_short", "1".repeat(63)),
        ("s3_upper", "AB".repeat(32)),
        ("s3_not_hex", "g".repeat(64)),
    ] {
        let s = Setup::new(name);
        s.write_json("invitations.json", &json!([invitation_entry(&bad)]));
        let result = s.run_to_exit();
        assert_eq!(
            assert_one_error(&result),
            "crpg-server: error invitations: entry 0: credential is not 64 lowercase hex",
            "{name}"
        );
        assert_eq!(s.sidecar(), "7\n", "{name}");
    }

    // Entry numbering: the second entry is the bad one.
    let s = Setup::new("s3_entry_1");
    s.write_json(
        "invitations.json",
        &json!([
            invitation_entry(&hex(&x())),
            invitation_entry(&"1".repeat(63))
        ]),
    );
    let result = s.run_to_exit();
    assert_eq!(
        assert_one_error(&result),
        "crpg-server: error invitations: entry 1: credential is not 64 lowercase hex"
    );

    // Control: the unmodified set-up reaches the start line.
    let s = Setup::new("s3_control");
    let mut server = Server::spawn(&s.config_path(), false);
    let start = server.start();
    assert!(start.line.contains("incarnation=8"), "{}", start.line);
    server.kill();
    assert_eq!(s.sidecar(), "8\n");
}

/// S4 (B-11 stages 6-7, B-12): a refused load has spent one incarnation.
#[test]
fn load_refusal_spends_one_incarnation() {
    let s = Setup::new("s4");
    let mut config = s.config();
    config["campaign_id"] = json!(u(2));
    s.write_config(&config);
    let wrong = SaveIdentity {
        campaign_id: u(2),
        campaign_version: "1.0.0".to_owned(),
    };
    let expected_8 = load_host_file(&s.path("world.save"), &wrong, 8, 0)
        .expect_err("a different campaign is refused")
        .to_string();
    let result = s.run_to_exit();
    assert_eq!(
        assert_one_error(&result),
        format!("crpg-server: error load: {expected_8}")
    );
    assert_eq!(s.sidecar(), "8\n");

    // A second run spends 9.
    let expected_9 = load_host_file(&s.path("world.save"), &wrong, 9, 0)
        .expect_err("still refused")
        .to_string();
    let result = s.run_to_exit();
    assert_eq!(
        assert_one_error(&result),
        format!("crpg-server: error load: {expected_9}")
    );
    assert_eq!(s.sidecar(), "9\n");

    // The sidecar deleted: the first incarnation is 1, which is not above the
    // saved 7, so the right identity is refused by the loader.
    let s = Setup::new("s4_deleted");
    std::fs::remove_file(s.path("world.save.incarnation")).expect("deletes the sidecar");
    let expected_1 = load_host_file(&s.path("world.save"), &id(), 1, 0)
        .expect_err("an incarnation not above the saved one")
        .to_string();
    let result = s.run_to_exit();
    assert_eq!(
        assert_one_error(&result),
        format!("crpg-server: error load: {expected_1}")
    );
    assert_eq!(s.sidecar(), "1\n");

    // A sidecar that is not a decimal number is refused before anything is
    // spent.
    let s = Setup::new("s4_unreadable");
    std::fs::write(s.path("world.save.incarnation"), "seven\n").expect("writes");
    let result = s.run_to_exit();
    assert_eq!(
        assert_one_error(&result),
        "crpg-server: error incarnation: unreadable sidecar"
    );
    assert_eq!(s.sidecar(), "seven\n");

    // A sidecar without its trailing newline is still read.
    let s = Setup::new("s4_no_newline");
    std::fs::write(s.path("world.save.incarnation"), "7").expect("writes");
    let mut server = Server::spawn(&s.config_path(), false);
    let start = server.start();
    assert!(start.line.contains("incarnation=8"), "{}", start.line);
    server.kill();
    assert_eq!(
        s.sidecar(),
        "8
"
    );

    // The last value cannot be spent.
    let s = Setup::new("s4_exhausted");
    std::fs::write(s.path("world.save.incarnation"), format!("{}\n", u64::MAX)).expect("writes");
    let result = s.run_to_exit();
    assert_eq!(
        assert_one_error(&result),
        "crpg-server: error incarnation: exhausted"
    );
    assert_eq!(s.sidecar(), format!("{}\n", u64::MAX));

    // Control: the right identity with the sidecar reaches the start line.
    let s = Setup::new("s4_control");
    let mut server = Server::spawn(&s.config_path(), false);
    let start = server.start();
    assert!(start.line.contains("incarnation=8"), "{}", start.line);
    server.kill();
}

// ---------------------------------------------------------------------------
// S5, S6.
// ---------------------------------------------------------------------------

/// S5 (B-11, B-13, B-15, B-16): the server serves, logs one line per
/// rejection to standard error and the appended log file, answers the
/// console, and on `quit` saves the world at its incarnation.
#[test]
fn serves_logs_and_saves_on_quit() {
    let s = Setup::new("s5");
    let mut config = s.config();
    config["log_file"] = json!("server.log");
    s.write_config(&config);
    std::fs::write(s.path("server.log"), "old\n").expect("pre-fills the log");

    let mut server = Server::spawn(&s.config_path(), true);
    let start = server.start();
    assert_eq!(start.line, start_line(start.port, 8, "none", "off"));

    // A hello with Y is refused and logged.
    match hello(start.port, &y()) {
        Err(ConnectError::Refused { code: 5 }) => {}
        Err(other) => panic!("expected a refusal with code 5, got {other:?}"),
        Ok(_) => panic!("expected a refusal, got a connection"),
    }
    let reject = server.next_line();
    let rest = reject
        .strip_prefix("crpg-server: reject t=")
        .unwrap_or_else(|| panic!("expected a reject line, got {reject:?}"));
    let (t, text) = rest.split_once(' ').expect("a time and a rejection");
    assert!(
        t.parse::<u64>().is_ok_and(|t| t < 30_000),
        "t is the loop time in ms from 0: {reject:?}"
    );
    assert_eq!(text, "hello conn=1 code=5");

    // Control: a hello with X adds no reject line (the next line is the
    // console's answer).
    let _client = hello(start.port, &x()).expect("X is accepted");
    server.send("status\n");
    assert_eq!(
        server.next_line(),
        "crpg-server: console: unknown command status"
    );
    // Blank lines are ignored and whitespace is trimmed.
    server.send("\n  \r\nwho  \r\n");
    assert_eq!(
        server.next_line(),
        "crpg-server: console: unknown command who"
    );
    // Leading padding is trimmed the same as trailing (B-15).
    server.send("  padded\n\t padded2 \r\n");
    assert_eq!(
        server.next_line(),
        "crpg-server: console: unknown command padded"
    );
    assert_eq!(
        server.next_line(),
        "crpg-server: console: unknown command padded2"
    );
    // `quit` is matched whole: neither `quit now` nor `quitx` stops the
    // server (B-15); the later `quit` does.
    server.send("quit now\nquitx\n");
    assert_eq!(
        server.next_line(),
        "crpg-server: console: unknown command quit now"
    );
    assert_eq!(
        server.next_line(),
        "crpg-server: console: unknown command quitx"
    );
    assert!(
        server.child.try_wait().expect("try_wait").is_none(),
        "only an exact `quit` stops the server"
    );

    server.send("quit\n");
    let status = server.wait();
    assert_eq!(status.code(), Some(0));
    let lines = server.rest();
    assert_eq!(
        lines.last().expect("a last line"),
        &format!(
            "crpg-server: stop reason=quit saved={}",
            s.path("world.save").display()
        )
    );

    // The log file is `old` followed by exactly the lines from the start line.
    let mut expected = String::from("old\n");
    for line in &lines {
        expected.push_str(line);
        expected.push('\n');
    }
    assert_eq!(
        std::fs::read_to_string(s.path("server.log")).expect("log reads"),
        expected
    );
    assert_no_credential(&lines);

    // The world was saved at incarnation 8.
    assert!(s.loads_at(9));
    assert!(!s.loads_at(8));
    assert_eq!(s.sidecar(), "8\n");
}

/// S6 (B-15, B-17): end of input does not stop the server.
#[test]
fn end_of_input_keeps_serving() {
    let s = Setup::new("s6");
    // A log file that does not exist yet is created (B-11 stage 2).
    let mut config = s.config();
    config["log_file"] = json!("new.log");
    s.write_config(&config);
    let mut server = Server::spawn(&s.config_path(), false);
    let start = server.start();
    let _client = hello(start.port, &x()).expect("X is accepted");
    std::thread::sleep(Duration::from_millis(300));
    assert!(server.child.try_wait().expect("try_wait").is_none());
    server.kill();
    assert_eq!(
        std::fs::read_to_string(s.path("new.log")).expect("log reads"),
        format!(
            "{}
",
            start.line
        )
    );
}

// ---------------------------------------------------------------------------
// S7, S8, S9.
// ---------------------------------------------------------------------------

/// S7 (B-11 stage 8): bounds that bake bind before serving; bounds that do not
/// refuse with the sidecar spent.
#[test]
fn area_binds_before_serving() {
    let s = Setup::new("s7");
    let mut config = s.config();
    config["area"] = json!({"min": [0, 0, 0], "max": [16, 0, 16]});
    s.write_config(&config);
    let mut server = Server::spawn(&s.config_path(), false);
    let start = server.start();
    assert_eq!(start.line, start_line(start.port, 8, "bound", "off"));
    server.kill();

    let s = Setup::new("s7_bad");
    let mut config = s.config();
    config["area"] = json!({"min": [16, 0, 16], "max": [0, 0, 0]});
    s.write_config(&config);
    let result = s.run_to_exit();
    let line = assert_one_error(&result);
    assert!(
        line.starts_with("crpg-server: error area: NavBake"),
        "{line}"
    );
    assert_eq!(s.sidecar(), "8\n");
}

/// The AI file of a, b, c with `controlled` b.
fn ai_doc(sides: Value) -> Value {
    json!({
        "sides": sides,
        "controlled": [{"index": 1, "generation": 1}]
    })
}

/// S8 (B-11 stage 5, B-14): a sides list serves with `ai=on`; a duplicate is
/// refused before any incarnation is spent.
#[test]
fn ai_file() {
    let entry =
        |index: u32, side: u32| json!({"entity": {"index": index, "generation": 1}, "side": side});

    let s = Setup::new("s8");
    s.write_json(
        "sides.json",
        &ai_doc(json!([entry(0, 1), entry(1, 2), entry(2, 1)])),
    );
    let mut config = s.config();
    config["ai"] = json!("sides.json");
    s.write_config(&config);
    let mut server = Server::spawn(&s.config_path(), false);
    let start = server.start();
    assert_eq!(start.line, start_line(start.port, 8, "none", "on"));
    server.kill();

    let s = Setup::new("s8_duplicate");
    s.write_json(
        "sides.json",
        &ai_doc(json!([entry(0, 1), entry(0, 2), entry(2, 1)])),
    );
    let mut config = s.config();
    config["ai"] = json!("sides.json");
    s.write_config(&config);
    let result = s.run_to_exit();
    let line = assert_one_error(&result);
    assert!(line.starts_with("crpg-server: error ai: "), "{line}");
    assert_eq!(s.sidecar(), "7\n");
}

/// S9 (B-11 stage 12): a duplicate credential is refused at registration.
#[test]
fn register_refusal() {
    let s = Setup::new("s9");
    s.write_json(
        "invitations.json",
        &json!([invitation_entry(&hex(&x())), invitation_entry(&hex(&x()))]),
    );
    let result = s.run_to_exit();
    assert_eq!(
        assert_one_error(&result),
        "crpg-server: error register: entry 1: DuplicateCredential at host/quic"
    );
    assert_eq!(s.sidecar(), "8\n");
}

// ---------------------------------------------------------------------------
// S10: signals (Unix; Windows has no automated case, Reading 13).
// ---------------------------------------------------------------------------

/// One signal case: serve, accept `X`, send `signal`, expect the saving stop.
#[cfg(unix)]
fn signal_case(name: &str, signal: &str, open_stdin: bool) {
    let s = Setup::new(name);
    let mut server = Server::spawn(&s.config_path(), open_stdin);
    let start = server.start();
    let _client = hello(start.port, &x()).expect("X is accepted");
    let status = Command::new("kill")
        .args(["-s", signal, &server.child.id().to_string()])
        .status()
        .expect("runs kill");
    assert!(status.success(), "kill -s {signal}");
    let exit = server.wait();
    assert_eq!(exit.code(), Some(0), "{signal}: {:?}", server.seen);
    let lines = server.rest();
    assert_eq!(
        lines.last().expect("a last line"),
        &format!(
            "crpg-server: stop reason=signal saved={}",
            s.path("world.save").display()
        ),
        "{signal}"
    );
    assert!(s.loads_at(9), "{signal}: the save reloads at 9");
    assert!(!s.loads_at(8), "{signal}: the save was rewritten at 8");
}

/// S10 (B-16, B-17): SIGTERM, SIGINT and SIGHUP stop the server through the
/// same save as `quit`, with standard input open and with it closed at spawn.
/// Control: S6, where no signal is sent and the server keeps running.
#[cfg(unix)]
#[test]
fn signals_stop_and_save() {
    for (name, signal) in [("s10_term", "TERM"), ("s10_int", "INT"), ("s10_hup", "HUP")] {
        signal_case(name, signal, true);
    }
    signal_case("s10_term_closed", "TERM", false);
}

// ---------------------------------------------------------------------------
// S11: PEM and DER.
// ---------------------------------------------------------------------------

/// Standard base64 (RFC 4648, `=` padding).
fn base64(data: &[u8]) -> String {
    const DIGITS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(char::from(DIGITS[(n >> 18) as usize & 63]));
        out.push(char::from(DIGITS[(n >> 12) as usize & 63]));
        out.push(if chunk.len() > 1 {
            char::from(DIGITS[(n >> 6) as usize & 63])
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            char::from(DIGITS[n as usize & 63])
        } else {
            '='
        });
    }
    out
}

/// A PEM block: `label`, 64-character lines, each ending `\r\n`.
fn pem(label: &str, der: &[u8]) -> Vec<u8> {
    let mut out = format!("-----BEGIN {label}-----\r\n");
    for line in base64(der).as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(line).expect("ascii"));
        out.push_str("\r\n");
    }
    out.push_str(&format!("-----END {label}-----\r\n"));
    out.into_bytes()
}

/// Runs a set-up whose certificate and key files are `cert` and `key`; the
/// start line must carry the fixture's pin and `X` must be accepted.
fn accepts_files(name: &str, cert: &[u8], key: &[u8]) {
    let s = Setup::new(name);
    std::fs::write(s.path("server.cert.der"), cert).expect("cert");
    std::fs::write(s.path("server.key.pk8.der"), key).expect("key");
    let mut server = Server::spawn(&s.config_path(), false);
    let start = server.start();
    assert_eq!(
        start.line,
        start_line(start.port, 8, "none", "off"),
        "{name}"
    );
    let _client = hello(start.port, &x()).unwrap_or_else(|e| panic!("{name}: {e:?}"));
    server.kill();
}

/// Runs a set-up whose files are `cert` and `key`; returns the one error line
/// and asserts the sidecar is as `sidecar` says.
fn refuses_files(name: &str, cert: &[u8], key: &[u8], sidecar: &str) -> String {
    let s = Setup::new(name);
    std::fs::write(s.path("server.cert.der"), cert).expect("cert");
    std::fs::write(s.path("server.key.pk8.der"), key).expect("key");
    let result = s.run_to_exit();
    let line = assert_one_error(&result).to_owned();
    assert_eq!(s.sidecar(), sidecar, "{name}");
    line
}

/// S11 (B-19): PEM and DER, told apart by the header, give the same server;
/// malformed PEM is refused before an incarnation is spent.
#[test]
fn pem_and_der() {
    // The helper is the crate's own encoding, read back.
    assert_eq!(
        CertificateDer::from_pem_slice(&pem("CERTIFICATE", rig::SERVER_A_CERT))
            .expect("pem certificate")
            .as_ref(),
        rig::SERVER_A_CERT
    );
    assert_eq!(
        PrivatePkcs8KeyDer::from_pem_slice(&pem("PRIVATE KEY", rig::SERVER_A_KEY))
            .expect("pem key")
            .secret_pkcs8_der(),
        rig::SERVER_A_KEY
    );

    let cert_pem = pem("CERTIFICATE", rig::SERVER_A_CERT);
    let key_pem = pem("PRIVATE KEY", rig::SERVER_A_KEY);
    let mut spaced_cert = b"\r\n  ".to_vec();
    spaced_cert.extend_from_slice(&cert_pem);

    accepts_files("s11_pem_pem", &spaced_cert, &key_pem);
    accepts_files("s11_pem_der", &cert_pem, rig::SERVER_A_KEY);
    accepts_files("s11_der_pem", rig::SERVER_A_CERT, &key_pem);

    // (m1) a key where the certificate should be.
    assert_eq!(
        refuses_files(
            "s11_m1",
            &pem("PRIVATE KEY", rig::SERVER_A_KEY),
            rig::SERVER_A_KEY,
            "7\n"
        ),
        "crpg-server: error certificate: certificate file: no items found"
    );
    // (m2) a PKCS#1 key is not converted.
    assert_eq!(
        refuses_files(
            "s11_m2",
            rig::SERVER_A_CERT,
            &pem("RSA PRIVATE KEY", rig::SERVER_A_KEY),
            "7\n"
        ),
        "crpg-server: error certificate: private_key file: no items found"
    );
    // (m3) a certificate with no end line.
    let truncated = b"-----BEGIN CERTIFICATE-----\r\nMIIB\r\n";
    let e = CertificateDer::from_pem_slice(truncated)
        .expect_err("a block with no end line does not decode")
        .to_string();
    assert!(e.starts_with("missing section end marker"), "{e}");
    assert_eq!(
        refuses_files("s11_m3", truncated, rig::SERVER_A_KEY, "7\n"),
        format!("crpg-server: error certificate: certificate file: {e}")
    );

    // Control: a DER file of the single byte 0x00 is never PEM-decoded: it
    // passes stage 3 and fails at `bind`, with the incarnation spent.
    let line = refuses_files("s11_der_zero", &[0x00], rig::SERVER_A_KEY, "8\n");
    assert!(line.starts_with("crpg-server: error bind: "), "{line}");
}
// ---------------------------------------------------------------------------
// Fix round A: boundaries, stage words and log sinks the first suite left
// to the mutant run (B-11 stages 1-4, B-13, B-16.3).
// ---------------------------------------------------------------------------

/// Spawns the server over `s` (no standard input) and reads its start line.
fn start_with(s: &Setup) -> (Server, Start) {
    let mut server = Server::spawn(&s.config_path(), false);
    let start = server.start();
    (server, start)
}

/// B-11.1: `-32768` and `32767` are inside the area range, so the range check
/// of stage 1 passes them and the failure is the later `area` stage's (the
/// fixture's movers sit at the origin, off a far-away mesh: `NavBake`, with
/// the incarnation spent); `32768` and `-32769` are outside, a stage-1 failure
/// that spends nothing. Control: the S3 cases (`40000`, `-32769`).
#[test]
fn area_range_boundaries() {
    for (name, min, max) in [
        ("area_hi", [32751, 0, 32751], [32767, 0, 32767]),
        ("area_lo", [-32768, 0, -32768], [-32752, 0, -32752]),
    ] {
        let s = Setup::new(name);
        let mut config = s.config();
        config["area"] = json!({"min": min, "max": max});
        s.write_config(&config);
        let result = s.run_to_exit();
        let line = assert_one_error(&result);
        assert!(
            line.starts_with("crpg-server: error area: NavBake"),
            "{name}: {line}"
        );
        assert_eq!(
            s.sidecar(),
            "8
",
            "{name}"
        );
    }
    for (name, min, max) in [
        ("area_over_max", [0, 0, 0], [32768, 0, 0]),
        ("area_under_min", [-32769, 0, 0], [0, 0, 0]),
    ] {
        let s = Setup::new(name);
        let mut config = s.config();
        config["area"] = json!({"min": min, "max": max});
        s.write_config(&config);
        let result = s.run_to_exit();
        assert_eq!(
            assert_one_error(&result),
            "crpg-server: error config: area value out of range",
            "{name}"
        );
        assert_eq!(s.sidecar(), "7\n", "{name}");
    }
}

/// B-11.4: exactly `MAX_INVITATIONS` (64) invitations start and the start
/// line counts them; two read `invitations=2`. Control: S3 refuses 65.
#[test]
fn invitation_counts_in_the_start_line() {
    for count in [2u8, 64] {
        let s = Setup::new(&format!("count_{count}"));
        let entries: Vec<Value> = (1..=count)
            .map(|i| invitation_entry(&hex(&[i; 32])))
            .collect();
        s.write_json("invitations.json", &Value::Array(entries));
        let (mut server, start) = start_with(&s);
        assert_eq!(
            start.line,
            start_line(start.port, 8, "none", "off")
                .replace("invitations=1", &format!("invitations={count}")),
            "{count}"
        );
        server.kill();
    }
}

/// B-11.2: a log file that cannot be opened is a stage-`log` failure with the
/// sidecar unspent. Control: S6's new log file opens.
#[test]
fn unopenable_log_file_is_a_log_error() {
    let s = Setup::new("log_dir");
    std::fs::create_dir(s.path("logdir")).expect("a directory where the log file goes");
    let mut config = s.config();
    config["log_file"] = json!("logdir");
    s.write_config(&config);
    let result = s.run_to_exit();
    let line = assert_one_error(&result);
    assert!(line.starts_with("crpg-server: error log: "), "{line}");
    assert_eq!(s.sidecar(), "7\n");
}

/// B-11.3: a certificate or key file that cannot be read gives the stage-3
/// line `{what} file: {io error}`, with the sidecar unspent. Control: S3's
/// unmodified set-up reaches the start line.
#[test]
fn missing_certificate_and_key_files() {
    for (name, file, what) in [
        ("cert_missing", "server.cert.der", "certificate"),
        ("key_missing", "server.key.pk8.der", "private_key"),
    ] {
        let s = Setup::new(name);
        std::fs::remove_file(s.path(file)).expect("removes the file");
        let result = s.run_to_exit();
        let line = assert_one_error(&result);
        let detail = line
            .strip_prefix(&format!("crpg-server: error certificate: {what} file: "))
            .unwrap_or_else(|| panic!("{name}: {line}"));
        assert!(!detail.is_empty(), "{name}: the I/O error follows");
        assert_eq!(s.sidecar(), "7\n", "{name}");
    }
}

/// Every log line of a start-up failure reaches the log file once it is open
/// (B-11 stage 2 on). Control: a failure at stage 1, before the file is
/// opened, creates no log file.
#[test]
fn startup_failure_line_is_in_the_log_file() {
    let s = Setup::new("log_failure");
    std::fs::write(
        s.path("server.cert.der"),
        pem("PRIVATE KEY", rig::SERVER_A_KEY),
    )
    .expect("a key where the certificate should be");
    let mut config = s.config();
    config["log_file"] = json!("fail.log");
    s.write_config(&config);
    let result = s.run_to_exit();
    let line = assert_one_error(&result).to_owned();
    assert_eq!(
        line,
        "crpg-server: error certificate: certificate file: no items found"
    );
    assert_eq!(
        std::fs::read_to_string(s.path("fail.log")).expect("the log file exists"),
        format!("{line}\n")
    );

    let s = Setup::new("log_failure_early");
    let mut config = s.config();
    config["log_file"] = json!("early.log");
    config["area"] = json!({"min": [40000, 0, 0], "max": [64, 0, 64]});
    s.write_config(&config);
    let result = s.run_to_exit();
    assert_one_error(&result);
    assert!(
        !s.path("early.log").exists(),
        "stage 1 fails before the log file is opened"
    );
}

/// The reject line's `{t}` and the text after it.
fn reject_parts(line: &str) -> (u64, &str) {
    let rest = line
        .strip_prefix("crpg-server: reject t=")
        .unwrap_or_else(|| panic!("expected a reject line, got {line:?}"));
    let (t, text) = rest.split_once(' ').expect("a time and a rejection");
    (t.parse().expect("t is a number"), text)
}

/// B-13: every rejection of a report is a line of its own, and `t` is the
/// loop's time, not a constant. Two refused hellos in two cycles give
/// increasing `t`; a burst of undecodable frames gives exactly one line per
/// frame however the pumps split them (a logger that keeps one line per
/// report loses some of the burst). Control: nothing else is logged for the
/// burst, and S5's accepted `X` adds no line.
#[test]
fn every_rejection_is_a_line_with_the_loop_time() {
    const BURST: usize = 12;
    let s = Setup::new("reject_lines");
    let (mut server, start) = start_with(&s);

    let mut times = Vec::new();
    for _ in 0..2 {
        assert!(matches!(
            hello(start.port, &y()),
            Err(ConnectError::Refused { code: 5 })
        ));
        let line = server.next_line();
        let (t, text) = reject_parts(&line);
        assert!(text.starts_with("hello conn="), "{line}");
        times.push(t);
    }
    assert!(times[0] > 0, "t is the loop time, not 0: {times:?}");
    assert!(
        times[1] > times[0],
        "a later cycle has a later t: {times:?}"
    );
    server.kill();

    // The burst, over a fresh server whose standard input is open.
    let s = Setup::new("reject_burst");
    let mut server = Server::spawn(&s.config_path(), true);
    let start = server.start();
    let mut client = hello(start.port, &x()).expect("X is accepted");
    for _ in 0..BURST {
        client.try_send(&[0xFF]).expect("client send");
    }
    for n in 0..BURST {
        let line = server.next_line();
        let (t, text) = reject_parts(&line);
        assert!(t > 0, "{line}");
        assert!(
            text.starts_with("admission peer=") && text.ends_with(" seq=- code=3 cached=false"),
            "rejection {n}: {line}"
        );
    }
    server.send("status\n");
    assert_eq!(
        server.next_line(),
        "crpg-server: console: unknown command status"
    );
}

/// B-11.9: the server unpauses the restored host before serving, so its loop
/// ticks the world. Control: the saved world before the run is at the
/// fixture's tick.
#[test]
fn served_world_ticks() {
    let s = Setup::new("ticks");
    let before = load_host_file(&s.path("world.save"), &id(), 8, 0)
        .expect("the set-up save loads")
        .host
        .server_tick();
    assert_eq!(before, rig::new_host(rig::V2).server_tick());

    let mut server = Server::spawn(&s.config_path(), true);
    let start = server.start();
    // A decided hello is a finished cycle: its advance step has run.
    let _client = hello(start.port, &x()).expect("X is accepted");
    server.send("quit\n");
    assert_eq!(server.wait().code(), Some(0));
    let after = load_host_file(&s.path("world.save"), &id(), 9, 0)
        .expect("the served save loads")
        .host
        .server_tick();
    assert!(
        after > before,
        "the served world ticked: {before} -> {after}"
    );
    let _ = start;
}

/// B-16.3: a save that cannot be written is `error save`, exit 1, and no stop
/// line. The save path is made a directory after the start line, which the
/// atomic replace cannot rename onto on Windows or Linux (a read-only file
/// would not stop a rename on Linux). Control: S5's quit saves.
#[test]
fn failed_save_exits_one() {
    let s = Setup::new("save_fails");
    let mut server = Server::spawn(&s.config_path(), true);
    let _start = server.start();
    std::fs::remove_file(s.path("world.save")).expect("removes the save");
    std::fs::create_dir(s.path("world.save")).expect("a directory where the save goes");
    server.send("quit\n");
    let status = server.wait();
    let lines = server.rest();
    assert_eq!(status.code(), Some(1), "{lines:?}");
    let last = lines.last().expect("a last line");
    assert!(last.starts_with("crpg-server: error save: "), "{lines:?}");
    assert!(
        lines.iter().all(|l| !l.starts_with("crpg-server: stop ")),
        "{lines:?}"
    );
}
