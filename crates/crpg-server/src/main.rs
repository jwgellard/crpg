#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! The dedicated server binary (T040b): the OS adapter over the
//! `crpg_server` library, which owns the authoritative host and T154's run
//! loop. It reads one JSON configuration file, loads a host save, binds the
//! area's navigation mesh, starts the resumable QUIC adapter, and hosts the
//! loop on the runner thread. It writes one log line for every rejected client
//! message (through the library's intake observer, T040a) to standard error
//! and to an optional log file, and on the console command `quit`, Ctrl-C or a
//! stop signal it stops the loop, closes the adapter and saves the world back
//! to its save file.
//!
//! The binary reads no clock: the loop starts at `START_MS` (Reading 2). It
//! creates no thread: the main thread reads the console, the runner thread is
//! T154's and the signal handler's is the `ctrlc` crate's (Reading 13). The
//! exact contract is `tasks/T040.md`.

use std::ffi::OsString;
use std::fmt::Display;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crpg_ai::{SideId, Sides};
use crpg_core::{EntityId, Fx16_16, Ulid};
use crpg_net::facts_v3::{EncounterGrants, HostGrants};
use crpg_net::protocol_v3::PauseRights;
use crpg_net_quic::{Credential, QuicServer, ServerConfig, ServerIdentity, ServerLimits};
use crpg_server::host::{
    ControlGrant, DisclosureGrants, EntityDisclosure, EntityFactGrant, FactGrants, HostError,
    MVP_AI_BUDGET,
};
use crpg_server::quic::{Invitation, QuicHost, V4Invitation, MAX_INVITATIONS};
use crpg_server::run::{HostLoop, QuicAiHook, QuicHosting};
use crpg_server::runner::{self, RunnerHandle, SystemClock};
use crpg_server::save::{load_host_file, save_host_file, SaveIdentity};
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use serde::Deserialize;

/// The time the host is loaded, started and looped at, and the origin of the
/// runner's clock (Reading 2): the binary never reads a clock.
const START_MS: u64 = 0;

/// The two usage lines `--help` prints (each ends with a newline).
const USAGE: &str = "Usage: crpg-server --config <path>\n       crpg-server --help | --version\n";

/// What the command line asks for.
enum Command {
    /// Print the usage and exit 0.
    Help,
    /// Print the version and exit 0.
    Version,
    /// Serve with the configuration file at this path.
    Run(PathBuf),
}

/// Reads the arguments after the program name. `--help` wins over
/// `--version`, which wins over the one accepted shape `--config <path>`;
/// the `Err` is the text after `crpg-server: usage: ` (B-10).
fn parse_args(args: Vec<OsString>) -> Result<Command, String> {
    if args.iter().any(|a| a == "--help") {
        return Ok(Command::Help);
    }
    if args.iter().any(|a| a == "--version") {
        return Ok(Command::Version);
    }
    let unknown = |a: &OsString| format!("unknown argument {}", a.to_string_lossy());
    match args.as_slice() {
        [] => Err("missing --config".to_owned()),
        [first, ..] if first != "--config" => Err(unknown(first)),
        [_] => Err("--config needs a value".to_owned()),
        [_, path] => Ok(Command::Run(PathBuf::from(path))),
        [_, _, extra, ..] => Err(unknown(extra)),
    }
}

/// One start-up or run failure: `stage` is a B-11 word, `detail` its text.
struct Failure {
    /// The start-up stage that failed.
    stage: &'static str,
    /// The failure's text.
    detail: String,
}

/// The closure that turns any displayable error into a failure at `stage`.
fn fail<E: Display>(stage: &'static str) -> impl FnOnce(E) -> Failure {
    move |error| Failure {
        stage,
        detail: error.to_string(),
    }
}

/// The sinks of [`Log`].
struct LogInner {
    /// The log file, once stage 2 opened it.
    file: Option<File>,
}

/// Standard error plus the optional log file, shared by the main thread, the
/// observer and the signal handler. Each line is one unbuffered `write_all`
/// per sink, so `process::exit` loses nothing; a failed write is ignored.
#[derive(Clone)]
struct Log(Arc<Mutex<LogInner>>);

impl Log {
    /// A log writing to standard error alone.
    fn new() -> Self {
        Log(Arc::new(Mutex::new(LogInner { file: None })))
    }

    /// From now on lines go to `file` too.
    fn attach(&self, file: File) {
        lock(&self.0).file = Some(file);
    }

    /// Writes `text` and a newline to every sink.
    fn line(&self, text: &str) {
        let mut bytes = Vec::with_capacity(text.len() + 1);
        bytes.extend_from_slice(text.as_bytes());
        bytes.push(b'\n');
        let mut inner = lock(&self.0);
        let _ = io::stderr().write_all(&bytes);
        if let Some(file) = inner.file.as_mut() {
            let _ = file.write_all(&bytes);
        }
    }
}

/// Locks `mutex`; a poisoned lock is used as is.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

// ---------------------------------------------------------------------------
// The configuration, invitations and AI files (JSON, `deny_unknown_fields`).
// ---------------------------------------------------------------------------

/// The configuration file.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    /// The UDP address to bind.
    listen: SocketAddr,
    /// The certificate file (DER, or PEM `CERTIFICATE`).
    certificate: PathBuf,
    /// The private key file (PKCS#8 DER, or PEM `PRIVATE KEY`).
    private_key: PathBuf,
    /// The invitations file.
    invitations: PathBuf,
    /// The save file; its `.incarnation` sidecar sits beside it.
    save: PathBuf,
    /// The campaign's stable id.
    campaign_id: Ulid,
    /// The campaign's published version text.
    campaign_version: String,
    /// The area's bounds, if the mesh is to be bound.
    area: Option<AreaConfig>,
    /// The AI sides file, if the server runs the AI.
    ai: Option<PathBuf>,
    /// The log file, if lines are to be kept.
    log_file: Option<PathBuf>,
    /// Serve the movement wire (protocol v4) instead of the combat wire
    /// (T155d); absent or `null` is `false`.
    movement: Option<bool>,
}

/// The area's bounds in whole world units.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AreaConfig {
    /// The least corner.
    min: [i32; 3],
    /// The greatest corner.
    max: [i32; 3],
}

/// One invitation of the invitations file.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InvitationEntry {
    /// 64 lowercase hex digits.
    credential: String,
    /// The actors the principal may act as.
    control: Vec<EntityId>,
    /// What the principal may learn.
    grants: GrantsEntry,
    /// The v4 fact grants (a movement world only, T155d).
    facts: Option<FactsEntry>,
    /// The principal's own pause rights (a movement world only, T155d).
    rights: Option<RightsEntry>,
}

/// `FactGrants`, key for key (T155d).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FactsEntry {
    /// Per-entity fact disclosure.
    entities: Vec<EntityFactEntry>,
    /// What may be learned of the encounter.
    encounter: EncounterEntry,
    /// What may be learned of the host's pause and timer.
    host: HostGrantsEntry,
}

/// `EntityFactGrant`, key for key (T155d).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EntityFactEntry {
    /// The whole entity id.
    entity: EntityId,
    /// Pools may be disclosed.
    pools: bool,
    /// Attached effects may be disclosed.
    effects: bool,
    /// Stat values may be disclosed.
    stats: bool,
    /// Record tags may be disclosed.
    tags: bool,
}

/// `EncounterGrants`, key for key (T155d).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EncounterEntry {
    /// The time mode may be disclosed.
    mode: bool,
    /// The pending mode may be disclosed.
    pending_mode: bool,
    /// The round clock may be disclosed.
    clock: bool,
    /// The timeline may be disclosed.
    timeline: bool,
}

/// `HostGrants`, key for key (T155d).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HostGrantsEntry {
    /// The pause reason may be disclosed.
    pause_reason: bool,
    /// The auto-pause toggles may be disclosed.
    auto_pause: bool,
    /// The turn timer may be disclosed.
    turn_timer: bool,
}

/// `PauseRights`, key for key (T155d).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RightsEntry {
    /// The principal may pause.
    pause: bool,
    /// The principal may unpause.
    unpause: bool,
}

/// `DisclosureGrants`, key for key.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GrantsEntry {
    /// Per-entity disclosure.
    entities: Vec<EntityGrantEntry>,
    /// Disclosable abilities.
    abilities: Vec<Ulid>,
    /// Disclosable encounters.
    encounters: Vec<Ulid>,
    /// Turn-start rounds may be disclosed.
    turn_round: bool,
    /// Natural encounter ends may be disclosed.
    encounter_end: bool,
    /// The turn state may be delivered.
    turn_state: bool,
}

/// `EntityDisclosure`, key for key.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EntityGrantEntry {
    /// The whole entity id.
    entity: EntityId,
    /// Visible and given a replica id.
    present: bool,
    /// Health may be delivered.
    health: bool,
    /// Spawn notices may be delivered.
    spawn: bool,
    /// Despawn notices may be delivered.
    despawn: bool,
    /// Death notices may be delivered.
    died: bool,
    /// May appear as an action's actor.
    action_actor: bool,
    /// May appear as an action's target.
    action_target: bool,
    /// Action outcomes may be disclosed.
    action_outcome: bool,
    /// Action damage may be disclosed.
    action_damage: bool,
    /// Turn starts may be disclosed.
    turn_start: bool,
}

/// The AI file.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AiFile {
    /// Who is on which side.
    sides: Vec<SideEntry>,
    /// The AI-controlled actors, passed to `drive_ai` as given.
    controlled: Vec<EntityId>,
}

/// One entry of the AI file's sides.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SideEntry {
    /// The entity.
    entity: EntityId,
    /// Its side.
    side: u32,
}

impl From<EntityGrantEntry> for EntityDisclosure {
    fn from(e: EntityGrantEntry) -> Self {
        EntityDisclosure {
            entity: e.entity,
            present: e.present,
            health: e.health,
            spawn: e.spawn,
            despawn: e.despawn,
            died: e.died,
            action_actor: e.action_actor,
            action_target: e.action_target,
            action_outcome: e.action_outcome,
            action_damage: e.action_damage,
            turn_start: e.turn_start,
        }
    }
}

impl From<GrantsEntry> for DisclosureGrants {
    fn from(g: GrantsEntry) -> Self {
        DisclosureGrants {
            entities: g.entities.into_iter().map(Into::into).collect(),
            abilities: g.abilities,
            encounters: g.encounters,
            turn_round: g.turn_round,
            encounter_end: g.encounter_end,
            turn_state: g.turn_state,
        }
    }
}

impl From<FactsEntry> for FactGrants {
    fn from(f: FactsEntry) -> Self {
        FactGrants {
            entities: f
                .entities
                .into_iter()
                .map(|e| EntityFactGrant {
                    entity: e.entity,
                    pools: e.pools,
                    effects: e.effects,
                    stats: e.stats,
                    tags: e.tags,
                })
                .collect(),
            encounter: EncounterGrants {
                mode: f.encounter.mode,
                pending_mode: f.encounter.pending_mode,
                clock: f.encounter.clock,
                timeline: f.encounter.timeline,
            },
            host: HostGrants {
                pause_reason: f.host.pause_reason,
                auto_pause: f.host.auto_pause,
                turn_timer: f.host.turn_timer,
            },
        }
    }
}

impl From<RightsEntry> for PauseRights {
    fn from(r: RightsEntry) -> Self {
        PauseRights {
            pause: r.pause,
            unpause: r.unpause,
        }
    }
}

/// One validated invitation, of the kind the world's wire takes (T155d).
enum Entry {
    /// A combat-wire (`/1`, `/2`) invitation.
    Combat(Invitation),
    /// A movement-wire (`/3`, v4) invitation.
    Movement(V4Invitation),
}

/// `path` joined to `base` when relative (an absolute `path` stays as it is).
fn resolve(base: &Path, path: &Path) -> PathBuf {
    base.join(path)
}

/// Exactly 64 characters of `0-9a-f` as 32 bytes.
fn parse_credential(text: &str) -> Option<Vec<u8>> {
    fn digit(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            _ => None,
        }
    }
    let bytes = text.as_bytes();
    if bytes.len() != 64 {
        return None;
    }
    let mut out = Vec::with_capacity(32);
    for pair in bytes.as_chunks::<2>().0.iter() {
        out.push((digit(pair[0])? << 4) | digit(pair[1])?);
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// Certificate and key bytes (B-19, Reading 7).
// ---------------------------------------------------------------------------

/// The PEM block `bytes` hold, from its `-----BEGIN` on, when they start one
/// after any leading ASCII whitespace. The decoder wants the header at the
/// start of a line, so the whitespace is dropped (S11's indented file).
fn pem_block(bytes: &[u8]) -> Option<&[u8]> {
    let start = bytes
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    let block = &bytes[start..];
    block.starts_with(b"-----BEGIN").then_some(block)
}

/// The DER bytes of a certificate file: PEM is decoded, anything else is DER
/// and returned unchanged (B-19).
fn certificate_der(bytes: Vec<u8>) -> Result<Vec<u8>, rustls_pki_types::pem::Error> {
    match pem_block(&bytes) {
        Some(block) => CertificateDer::from_pem_slice(block).map(|der| der.as_ref().to_vec()),
        None => Ok(bytes),
    }
}

/// The PKCS#8 DER bytes of a key file: PEM is decoded, anything else is DER
/// and returned unchanged (B-19).
fn private_key_der(bytes: Vec<u8>) -> Result<Vec<u8>, rustls_pki_types::pem::Error> {
    match pem_block(&bytes) {
        Some(block) => {
            PrivatePkcs8KeyDer::from_pem_slice(block).map(|key| key.secret_pkcs8_der().to_vec())
        }
        None => Ok(bytes),
    }
}

// ---------------------------------------------------------------------------
// Stopping (Readings 13 and 14).
// ---------------------------------------------------------------------------

/// Set by the signal handler; polled by the end-of-input wait (B-17).
static STOP: AtomicBool = AtomicBool::new(false);

/// What a stop needs; taken exactly once, by `finish` or `die`.
struct Running {
    /// The loop's owner.
    handle: RunnerHandle<QuicHosting>,
    /// Where the world is saved.
    save: PathBuf,
    /// The campaign identity the save carries.
    identity: SaveIdentity,
    /// Where lines go.
    log: Log,
}

/// The slot `finish` and `die` take the [`Running`] from.
type Shared = Arc<Mutex<Option<Running>>>;

/// Why the server stops.
#[derive(Clone, Copy)]
enum Reason {
    /// The console command `quit`.
    Quit,
    /// Ctrl-C or a stop signal.
    Signal,
}

impl Reason {
    /// The word the stop line carries.
    fn word(self) -> &'static str {
        match self {
            Reason::Quit => "quit",
            Reason::Signal => "signal",
        }
    }
}

/// B-16: the one stop routine. Locks `shared`, stops the loop, closes the
/// adapter, saves, and exits **while still holding the lock**, so whichever
/// of the console and a signal gets the lock first finishes and the other
/// blocks until the process ends. Never release the guard before the exit.
fn finish(shared: &Shared, reason: Reason) -> ! {
    let mut guard = lock(shared);
    let Some(running) = guard.take() else {
        std::process::exit(1);
    };
    let Running {
        handle,
        save,
        identity,
        log,
    } = running;
    let exit = match handle.join() {
        Ok(exit) => exit,
        Err(_) => {
            log.line("crpg-server: error loop: panicked");
            std::process::exit(1);
        }
    };
    if let Some(error) = exit.error {
        log.line(&format!("crpg-server: error loop: {error}"));
        std::process::exit(1);
    }
    let shutdown = exit.host_loop.into_hosting().into_quic().shutdown(false);
    match save_host_file(&save, &shutdown.host, &identity) {
        Ok(()) => {
            log.line(&format!(
                "crpg-server: stop reason={} saved={}",
                reason.word(),
                save.display()
            ));
            std::process::exit(0);
        }
        Err(error) => {
            log.line(&format!("crpg-server: error save: {error}"));
            std::process::exit(1);
        }
    }
}

/// B-17's loop-death exit, under the held guard: no save is attempted.
fn die(mut slot: MutexGuard<'_, Option<Running>>) -> ! {
    let Some(running) = slot.take() else {
        std::process::exit(1);
    };
    let log = running.log.clone();
    let line = match running.handle.join() {
        Ok(exit) => match exit.error {
            Some(error) => format!("crpg-server: error loop: {error}"),
            None => "crpg-server: error loop: ended".to_owned(),
        },
        Err(_) => "crpg-server: error loop: panicked".to_owned(),
    };
    log.line(&line);
    std::process::exit(1);
}

// ---------------------------------------------------------------------------
// Start-up (B-11).
// ---------------------------------------------------------------------------

/// The incarnation sidecar's path: `<save>.incarnation`.
fn sidecar_path(save: &Path) -> PathBuf {
    let mut name = save.as_os_str().to_owned();
    name.push(".incarnation");
    PathBuf::from(name)
}

/// Spends the next incarnation: reads the sidecar, writes `last + 1` and
/// returns it (stage 6).
fn spend_incarnation(save: &Path) -> Result<u64, Failure> {
    let path = sidecar_path(save);
    let last = match fs::read(&path) {
        Ok(bytes) => {
            let digits = bytes.strip_suffix(b"\n").unwrap_or(&bytes);
            if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
                return Err(fail("incarnation")("unreadable sidecar"));
            }
            std::str::from_utf8(digits)
                .ok()
                .and_then(|text| text.parse::<u64>().ok())
                .ok_or_else(|| fail("incarnation")("unreadable sidecar"))?
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => 0,
        Err(error) => return Err(fail("incarnation")(error)),
    };
    let next = last
        .checked_add(1)
        .ok_or_else(|| fail("incarnation")("exhausted"))?;
    let write = || -> io::Result<()> {
        let mut file = File::create(&path)?;
        file.write_all(format!("{next}\n").as_bytes())?;
        file.sync_all()
    };
    write().map_err(fail("incarnation"))?;
    Ok(next)
}

/// Reads and parses a JSON file.
fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

/// Starts the server: B-11 stages 1 to 16. On success the loop is running,
/// the handler is set and the start line is written.
fn serve(config_path: &Path, log: &Log) -> Result<Shared, Failure> {
    // 1. config.
    let config: Config = read_json(config_path).map_err(fail("config"))?;
    if let Some(area) = &config.area {
        let range = -32768..=32767;
        if area.min.iter().chain(&area.max).any(|v| !range.contains(v)) {
            return Err(fail("config")("area value out of range"));
        }
    }
    let movement = config.movement == Some(true);
    if movement && config.area.is_none() {
        return Err(fail("config")("movement needs area"));
    }
    let base = config_path.parent().unwrap_or_else(|| Path::new(""));
    let save = resolve(base, &config.save);

    // 2. log.
    if let Some(name) = &config.log_file {
        let file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(resolve(base, name))
            .map_err(fail("log"))?;
        log.attach(file);
    }

    // 3. certificate.
    let read = |name: &Path, what: &str| -> Result<Vec<u8>, Failure> {
        fs::read(resolve(base, name)).map_err(|e| Failure {
            stage: "certificate",
            detail: format!("{what} file: {e}"),
        })
    };
    let wrap = |what: &str| {
        let what = what.to_owned();
        move |e: rustls_pki_types::pem::Error| Failure {
            stage: "certificate",
            detail: format!("{what} file: {e}"),
        }
    };
    let cert =
        certificate_der(read(&config.certificate, "certificate")?).map_err(wrap("certificate"))?;
    let key =
        private_key_der(read(&config.private_key, "private_key")?).map_err(wrap("private_key"))?;
    let identity_key = ServerIdentity::new(cert, key);

    // 4. invitations.
    let entries: Vec<InvitationEntry> =
        read_json(&resolve(base, &config.invitations)).map_err(fail("invitations"))?;
    if entries.is_empty() {
        return Err(fail("invitations")("no invitations"));
    }
    if entries.len() > MAX_INVITATIONS {
        return Err(fail("invitations")("too many invitations"));
    }
    let mut invitations = Vec::with_capacity(entries.len());
    for (i, entry) in entries.into_iter().enumerate() {
        let credential = parse_credential(&entry.credential)
            .and_then(|bytes| Credential::new(bytes).ok())
            .ok_or_else(|| {
                fail("invitations")(format!("entry {i}: credential is not 64 lowercase hex"))
            })?;
        let control = ControlGrant {
            actors: entry.control,
        };
        let grants = entry.grants.into();
        invitations.push(match (movement, entry.facts, entry.rights) {
            (true, Some(facts), Some(rights)) => Entry::Movement(V4Invitation {
                credential,
                control,
                grants,
                facts: facts.into(),
                rights: rights.into(),
            }),
            (true, _, _) => {
                return Err(fail("invitations")(format!(
                    "entry {i}: movement needs facts and rights"
                )));
            }
            (false, None, None) => Entry::Combat(Invitation {
                credential,
                control,
                grants,
            }),
            (false, _, _) => {
                return Err(fail("invitations")(format!(
                    "entry {i}: facts and rights need movement"
                )));
            }
        });
    }

    // 5. ai.
    let ai = match &config.ai {
        Some(name) => {
            let file: AiFile = read_json(&resolve(base, name)).map_err(fail("ai"))?;
            let sides = Sides::new(
                file.sides
                    .into_iter()
                    .map(|s| (s.entity, SideId(s.side)))
                    .collect(),
            )
            .map_err(fail("ai"))?;
            Some((sides, file.controlled))
        }
        None => None,
    };

    // 6. incarnation.
    let incarnation = spend_incarnation(&save)?;

    // 7. load.
    let identity = SaveIdentity {
        campaign_id: config.campaign_id,
        campaign_version: config.campaign_version,
    };
    let mut host = load_host_file(&save, &identity, incarnation, START_MS)
        .map_err(fail("load"))?
        .host;

    // 8. area.
    if let Some(area) = &config.area {
        let corner = |v: [i32; 3]| v.map(Fx16_16::from_int);
        host.bind_area_bounds(corner(area.min), corner(area.max))
            .map_err(|e: HostError| fail("area")(e))?;
    }

    // 9. unpause.
    host.unpause(START_MS).map_err(fail("unpause"))?;

    // 10. bind.
    let pin = identity_key.pin().to_hex();
    let server_config = ServerConfig {
        bind: config.listen,
        identity: identity_key,
        limits: ServerLimits::v1(),
    };
    let server = if movement {
        QuicServer::bind_movement_resumable(server_config)
    } else {
        QuicServer::bind_resumable(server_config)
    }
    .map_err(fail("bind"))?;

    // 11. start.
    let mut quic =
        QuicHost::start_resumable(host, server, START_MS).map_err(|f| fail("start")(f.error))?;

    // 12. register.
    let count = invitations.len();
    for (i, invitation) in invitations.into_iter().enumerate() {
        match invitation {
            Entry::Combat(invitation) => quic.register_invitation(invitation).map(|_| ()),
            Entry::Movement(invitation) => quic.register_v4_invitation(invitation).map(|_| ()),
        }
        .map_err(|e| fail("register")(format!("entry {i}: {e}")))?;
    }

    // 13. hosting, hook and observer.
    let addr = quic.local_addr();
    let nav = quic.host().nav_revision().is_some();
    let mut hosting = QuicHosting::new(quic);
    let ai_on = ai.is_some();
    if let Some((sides, controlled)) = ai {
        let hook: QuicAiHook = Box::new(move |quic, _now| {
            quic.drive_ai(&sides, &controlled, MVP_AI_BUDGET)
                .map(|_report| ())
        });
        hosting.set_ai(Some(hook));
    }
    let observer_log = log.clone();
    hosting.set_intake_observer(Some(Box::new(move |now_ms, report| {
        for rejection in report.rejections() {
            observer_log.line(&format!("crpg-server: reject t={now_ms} {rejection}"));
        }
    })));

    // 14. spawn.
    let handle = runner::spawn(
        HostLoop::new(hosting, START_MS),
        SystemClock::starting_at(START_MS),
    )
    .map_err(fail("spawn"))?;
    let shared: Shared = Arc::new(Mutex::new(Some(Running {
        handle,
        save,
        identity,
        log: log.clone(),
    })));

    // 15. signals.
    let for_handler = Arc::clone(&shared);
    if let Err(error) = ctrlc::set_handler(move || {
        STOP.store(true, Ordering::SeqCst);
        finish(&for_handler, Reason::Signal)
    }) {
        log.line(&format!("crpg-server: error signals: {error}"));
        if let Some(running) = lock(&shared).take() {
            let _ = running.handle.join();
        }
        std::process::exit(1);
    }

    // 16. the start line.
    log.line(&format!(
        "crpg-server: start listen={addr} incarnation={incarnation} pin={pin} nav={} ai={} invitations={count}{}",
        if nav { "bound" } else { "none" },
        if ai_on { "on" } else { "off" },
        if movement { " movement=on" } else { "" },
    ));
    Ok(shared)
}

/// B-15 and B-17: reads the console until `quit`, and after the end of input
/// keeps serving until a signal or the loop's death. Never returns.
fn console(shared: &Shared, log: &Log) -> ! {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match input.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        {
            let guard = lock(shared);
            match guard.as_ref() {
                Some(running) if running.handle.is_finished() => die(guard),
                Some(_) => {}
                None => std::process::exit(1),
            }
        }
        let text = String::from_utf8_lossy(&buf);
        let command = text.trim_matches(|c: char| c.is_ascii_whitespace());
        if command == "quit" {
            finish(shared, Reason::Quit);
        } else if !command.is_empty() {
            log.line(&format!("crpg-server: console: unknown command {command}"));
        }
    }
    // End of input: keep serving; a signal or the loop's death ends this.
    loop {
        if STOP.load(Ordering::SeqCst) {
            finish(shared, Reason::Signal);
        }
        let guard = lock(shared);
        match guard.as_ref() {
            Some(running) => {
                if running.handle.call(|_| ()).is_err() {
                    die(guard);
                }
            }
            None => std::process::exit(1),
        }
    }
}

fn main() {
    let command = match parse_args(std::env::args_os().skip(1).collect()) {
        Ok(command) => command,
        Err(text) => {
            eprintln!("crpg-server: usage: {text}");
            std::process::exit(2);
        }
    };
    match command {
        Command::Help => {
            let _ = io::stdout().write_all(USAGE.as_bytes());
        }
        Command::Version => {
            let line = format!(
                "crpg-server {}
",
                env!("CARGO_PKG_VERSION")
            );
            let _ = io::stdout().write_all(line.as_bytes());
        }
        Command::Run(path) => {
            let log = Log::new();
            match serve(&path, &log) {
                Ok(shared) => console(&shared, &log),
                Err(failure) => {
                    log.line(&format!(
                        "crpg-server: error {}: {}",
                        failure.stage, failure.detail
                    ));
                    std::process::exit(1);
                }
            }
        }
    }
}
