# T040 — the dedicated `crpg-server` binary (`crpg-server`, `.github/workflows`)

**Status: blind draft A, not approved.** Written by a DeepSeek V4 Pro architect
from the fact sheet (`tasks/T040-facts.md`) and the binding inputs it cites:
ADR-0012 (`docs/adr/0012-windows-primary-platform.md`), D65a, D66b, D67a, the
root `AGENTS.md`, and the T040 row of `docs/IMPLEMENTATION_PLAN.md` (§3, M4
4E). A Claude architect writes the final contract; the user approves it.
Nothing here is approved. Any fact was re-checked in code; anything not
re-checked is cited only as "not verified" or left as an open question.

Citation convention (T140's): a bare `:N` or `:N–M` is a line of the file
cited earlier in the same item or sentence. Branch citations are
`<branch>:<path>:line`. Every citation was checked on `task/T040-draft-a-d1`
(from `task/T040-facts-f1`: `master` plus the fact sheet).

## Task

Turn `crpg-server`'s refuse-to-run binary into the dedicated server program.
Today `src/main.rs` prints `crpg-server: dedicated adapter not yet
implemented` and exits 1 (`crates/crpg-server/src/main.rs:9–12`); it is a
wiring shell over the `crpg_server` library, and no host policy lives in it
(`crates/crpg-server/AGENTS.md:24–26`). T040 replaces it with a real program
that, on Windows/MSVC and on a Godot-free Linux/GNU build, loads a
certificate and key, an invitation file, and a save, then hosts the T154 run
loop behind `QuicHosting` (`task/T154-c1:tasks/T154.md` Reading 13) and runs
it on the `runner` thread until a clean shutdown.

The pieces are already built and T040 only wires them (D66b, answer (a),
`tasks/DECISIONS-D66.md:17–25`): the run loop (`run.rs`), the clock and
thread (`runner.rs`), the QUIC adapter (`quic.rs`), the save adapter
(`save.rs`), and T154d's `QuicHosting`/`QuicAiHook` (contract only, not yet
built). What T040 adds is the operator surface the library deliberately
leaves out (`docs/architecture/crpg-server.md:26–33`): the command line, a
JSON configuration file, certificate/key loading, invitation registration,
the save identity and a never-decreasing incarnation source, logging — with
the D67a rejected-message log line and a "where do server logs go" decision
(`tasks/BACKLOG.md:341`, `tasks/DECISIONS-D67.md:5–12`) — and graceful
shutdown. It also wires the two filed follow-ups: `bind_area_bounds` at area
load and `place` after transfers and spawns (T037 Reading 5,
`tasks/T037.md:160–167`, `tasks/BACKLOG.md:344`), and the gate-12 dedicated
server artifact jobs (server half of `docs/CRPG_ENGINE_SPEC.md:1333`,
Windows plus Godot-free Linux, `:1345–1348`).

T040 is one crate (`crpg-server`) plus the workflow file, in two parts
below. No new dependency is added: argument parsing is hand-rolled over
`std::env::args`, configuration is JSON through the already-present
`serde_json` (`crates/crpg-server/Cargo.toml:32–33`), and logging is a
std-only line writer over `std::io`. A std-only alternative is given for
every place a parser or logger library would otherwise be tempting
(`AGENTS.md` "Do not add dependencies without being asked").

## Parts

Two parts, both S-set. T040a is S-set because the T040 row is `C → I, S`
(`docs/IMPLEMENTATION_PLAN.md:245`); T040b is S-set because
`.github/workflows/**` is in the serialized set (`docs/IMPLEMENTATION_PLAN.md:96`).
Order: **T040a then T040b** — T040b's artifact job builds and uploads the
binary T040a produces, so the job name and the binary name must be stable
before the workflow lands. T040a may run alone; T040b starts after T040a
merges.

| Part | Crate / tree | Scope | S-set |
|---|---|---|---|
| T040a | `crpg-server` | the binary: CLI, config, certificate/key, invitations, save + incarnation, logging (the D67a line and its destination), shutdown, hosting the T154 loop, `bind_area_bounds`/`place` wiring | **yes** (T040 row) |
| T040b | `.github/workflows/ci.yml` | the gate-12 dedicated-server artifact jobs (Windows/MSVC and Linux/GNU), no Rust | **yes** (workflows) |

True prerequisite: T154a–d merged (`QuicHosting`, `runner`, the loop), T039
(save), T023b (QUIC adapter), T153 (`Host::drive_ai`), T150c
(`QuicHost::advance`, `register_v4_invitation`). The plan's stale
`T023b` prerequisite (`docs/IMPLEMENTATION_PLAN.md:245`) predates the T154
run loop; the handoff is `task/T154-c1:tasks/T154.md:245–252`.

### T040a owns
- `crates/crpg-server/src/main.rs` (replaced: the entry, wiring and shutdown)
- `crates/crpg-server/src/cli.rs` (new, binary-only: `std::env::args` parsing)
- `crates/crpg-server/src/config.rs` (new, binary-only: JSON config load and
  validation over the existing `serde_json` edge)
- `crates/crpg-server/src/logline.rs` (new, binary-only: the std-only log
  writer and its line formats)
- `crates/crpg-server/src/quic.rs` (one additive item: `QuicHost::last_report`,
  Reading 8; a Manager edit to T023b/T154d's surface lists)
- `crates/crpg-server/tests/dedicated_binary.rs` (new: black-box binary cases
  and unit cases for the pure `cli`/`config`/`logline` parts)
- `docs/architecture/crpg-server.md` (the "planned and **not** here" list
  shrinks), `crates/crpg-server/AGENTS.md` (invariant 7 fence amended, the
  binary surface named, new traps)
- goldens: none new, none moved.

### T040b owns
- `.github/workflows/ci.yml` (one new `server-artifacts` job; no other
  workflow file is created or touched)
- goldens: none new, none moved.

## Readings

Design decisions this contract makes. Each needs approval with the contract.

1. **The binary is the OS adapter; the library's I/O fence stays.** ADR-0012
   item 3 puts OS-specific process/filesystem/service/presentation logic above
   `crpg-core`/`crpg-rules`/`crpg-sim` (`docs/adr/0012-windows-primary-platform.md:27–29`);
   the dedicated binary is that adapter. So the binary's own modules
   (`main.rs`, `cli.rs`, `config.rs`, `logline.rs`) may read the config,
   certificate, key and invitation files, write log lines and read the
   console with `std` directly. The *library* (`lib.rs` and its seven modules)
   keeps invariant 7's fence: filesystem I/O only in `save.rs`, network I/O
   only in `quic.rs`, one clock/thread only in `runner.rs`
   (`crates/crpg-server/AGENTS.md:135–143`). The binary reaches `Host` only
   through the library's public API. *Rejected:* reading config/certs/logs in
   the library (spreads operator I/O through the reusable host, against
   ADR-0012 item 3 and the E012 placement, `crates/crpg-server/AGENTS.md:24–26`).
2. **Argument parsing is hand-rolled over `std::env::args`, a pure function.**
   The flag set is the fixed, small list in "Interface"; a parser dependency
   needs approval (root `AGENTS.md` "Do not add dependencies"). `parse` takes
   an `Iterator<Item = String>`, so tests feed argument vectors without
   spawning the process, as the crate already black-box-tests
   (`crates/crpg-cli/tests/demo.rs:3–9`). *Rejected:* clap/pico-args (a new
   dependency for ~12 flags).
3. **Configuration is a JSON file over the existing `serde_json` edge; CLI
   flags override it.** Every `Cli` field has a `--flag` and a config key;
   a flag present on the command line wins over the file. Secrets are never
   inline: the file names paths (`cert`, `key`, `invitations`), never bytes.
   *Rejected:* TOML/YAML (a new parser); environment variables as the only
   channel (undiscoverable and hard to pin in tests).
4. **Logging is a std-only line writer; logs go to stderr by default, plus an
   optional file.** D67a's "where do server logs go" is answered this way:
   stderr by default (the headless server's stream), `--log-file <path>`
   appending the same lines to a file. Lines are newline-terminated UTF-8
   written through `&mut dyn io::Write`, so tests capture them to a `Vec<u8>`.
   No log crate, no levels, no rotation (out of scope). *Rejected:* a logging
   crate (new dependency); stdout for logs (stdout is reserved for the
   machine-readable `--headless-deterministic` state hash,
   `docs/CRPG_ENGINE_SPEC.md:958`).
5. **The rejected-message line is one fixed line per refused frame, from the
   pump report.** D67a: T042 pins the rejection and its text; T040 writes the
   line (`tasks/DECISIONS-D67.md:7–12`). `QuicPumpReport` already counts and
   traces every refusal (`frames_refused`, `calls`, `lane1`,
   `crates/crpg-server/src/quic.rs:99–120`). The binary writes one `reject`
   line per refused inbound lane-0 or lane-1 frame, in pump order, in the
   fixed format of "Interface". *Rejected:* logging only the count (spec:1488
   says "a log line" per message); logging every accepted message (noise).
6. **Shutdown is a console EOF or the line `quit` on the main thread.** The
   loop runs on `runner::spawn`'s thread (`crates/crpg-server/src/runner.rs:88`);
   the main thread blocks reading stdin. On EOF or `quit` it calls
   `RunnerHandle::stop()`, then `join()`, then a final quiescent
   `save_host_file` when `--save` is set, writes the `stop` line, and exits 0.
   This is std-only and platform-neutral. *Rejected:* a SIGINT/SIGTERM/Ctrl-C
   handler (on Windows a console-control handler needs `unsafe`, forbidden
   outside `crpg-godot` (`AGENTS.md:25–27`), and on Unix it needs a dependency
   or `unsafe`); see the open question.
7. **The incarnation is operator-supplied and guarded by a sidecar.** The
   loader's `new_incarnation` must exceed every incarnation ever used for that
   authority, or an older save would reuse epochs
   (`crates/crpg-server/AGENTS.md:300–302`). The binary takes `--incarnation`
   and, before `load_host_file`, reads `<save>.incarnation` (the last used
   value, `0` when absent): the supplied value must exceed it, else startup
   fails; after a successful load the sidecar is rewritten to the supplied
   value. *Rejected:* trusting the operator's memory (the trap is explicit);
   deriving the value from a wall clock (not monotonic within one second).
8. **Rejections reach the binary through `QuicHost::last_report`, drained from
   the AI hook.** The loop's `Hosting::intake` calls `QuicHost::pump` and drops
   the report (`task/T154-c1:tasks/T154.md:459–464`); the report is not stored
   today. T040 adds one additive item: `QuicHost` keeps the last
   `QuicPumpReport` (set at the end of `pump` and `flush`) and
   `pub fn last_report(&self) -> Option<&QuicPumpReport>`. The binary's
   `QuicAiHook` closure runs right after intake in the cycle order (intake,
   AI hook, advance, deliver; `tasks/T153.md` Reading 1), so it drains
   `last_report` into `reject` lines once per cycle. *Rejected:* a second
   `pump` to observe the report (it admits after the tick, T154 Reading 12);
   the report on `QuicHosting` (the hook receives `&mut QuicHost`, not the
   hosting).
9. **The dedicated server runs with no AI hook beyond logging in the MVP.**
   Driving `Host::drive_ai` needs `crpg_ai::Sides` built from authored data,
   and `crpg-server` reads no `crpg-data` (`crates/crpg-server/AGENTS.md:151–154`,
   `tasks/T153.md` Reading 3). The binary's `QuicAiHook` closure does the log
   drain (Reading 8) and nothing else; wiring `drive_ai` is a follow-up for
   the campaign-load task. *Rejected:* adding a `crpg-data` edge in T040 (a
   new dependency that reverses D20's "no `crpg-data`" decision for a smoke
   that needs no AI).
10. **`bind_area_bounds` at area load and `place` after transfers/spawns take
    operator-supplied bounds.** A restored host has `nav_revision() == None`
    and must be re-bound (`tasks/T037.md:145–152`); the mesh is not in the
    save. With no `crpg-data` edge, the area's two bounds are trusted operator
    facts, like `SaveIdentity` (`crates/crpg-server/src/save.rs:87–94`),
    supplied as `--area-min`/`--area-max`. When both are present the binary
    calls `Host::bind_area_bounds(min, max)` right after load, before
    unpausing and before the loop starts, and any mover the server itself
    spawns or transfers after binding is `place`d by it (T037 Reading 5: only
    binding places; later movers are the caller's,
    `tasks/T037.md:160–167`). With no bounds supplied the server runs
    navigation-free and movement requests return `NoNavMesh`, which is correct
    for a movement-free encounter and is not a startup error. *Rejected:* a
    `crpg-data` edge here (deferred to the campaign-load task); silently
    skipping the backlog row.

## Interface

Additions only; the library gains exactly one public item (Reading 8). Every
new item has a doc comment. Binary-only items are `pub(crate)`. No
`HashMap`/`HashSet`, no float, no `unsafe`, no new dependency; the binary's
I/O is plain `std` (Reading 1). `Fx16_16`, `EntityId`, `Ulid` are
`crpg_core`'s; `SocketAddr`, `PathBuf` are `std`'s; `ServerIdentity`,
`CertificatePin`, `Credential` are `crpg_net_quic`'s
(`crates/crpg-net-quic/src/identity.rs:76`, `:14`); `SaveIdentity`,
`Invitation`, `ControlGrant`, `DisclosureGrants` are `crpg_server`'s
(`crates/crpg-server/src/save.rs:89`, `crates/crpg-server/src/quic.rs:81`).

### The command line (`crates/crpg-server/src/cli.rs`)

| Flag | Value | Default | Notes |
|---|---|---|---|
| `--config <path>` | path | none | JSON file; every other flag overrides its key |
| `--listen <addr>` | `IP:port` | none (required) | UDP bind address |
| `--cert <path>` | path | none (required) | leaf certificate, DER |
| `--key <path>` | path | none (required) | PKCS#8 private key, DER |
| `--invitations <path>` | path | none (required) | invitation JSON file |
| `--save <path>` | path | none (required) | host save file |
| `--campaign-id <26 chars>` | ULID text | none (required) | `SaveIdentity.campaign_id` |
| `--campaign-version <text>` | 1..=64 of `0-9 A-Z a-z . + -` | none (required) | `SaveIdentity.campaign_version` |
| `--incarnation <u64>` | decimal | none (required) | new incarnation, guarded (Reading 7) |
| `--area-min <x,y,z>` | 3 ints | none | both or neither (Reading 10) |
| `--area-max <x,y,z>` | 3 ints | none | both or neither |
| `--log-file <path>` | path | none | append logs here too |
| `-h`, `--help` | — | — | print usage, exit 0 |
| `-V`, `--version` | — | — | print version, exit 0 |

```rust
pub(crate) const PROGRAM: &str = "crpg-server";
pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Where `Log` writes (Reading 4).
pub(crate) enum LogTo { Stderr, File(std::path::PathBuf), Both(std::path::PathBuf) }

pub(crate) struct Cli {
    pub(crate) config: Option<std::path::PathBuf>,
    pub(crate) listen: Option<std::net::SocketAddr>,
    pub(crate) cert: Option<std::path::PathBuf>,
    pub(crate) key: Option<std::path::PathBuf>,
    pub(crate) invitations: Option<std::path::PathBuf>,
    pub(crate) save: Option<std::path::PathBuf>,
    pub(crate) campaign_id: Option<String>,
    pub(crate) campaign_version: Option<String>,
    pub(crate) incarnation: Option<u64>,
    pub(crate) area_min: Option<[crpg_core::Fx16_16; 3]>,
    pub(crate) area_max: Option<[crpg_core::Fx16_16; 3]>,
    pub(crate) log_file: Option<std::path::PathBuf>,
}

/// What `parse` decided: run the server, or handle `--help`/`--version`.
pub(crate) enum CliAction { Run(Cli), Help, Version }

/// Parse an argument vector; `std::env::args().skip(1)` in production.
pub(crate) fn parse<I: Iterator<Item = String>>(args: I) -> Result<CliAction, CliError>;

pub(crate) enum CliError {
    UnknownFlag(String),
    MissingValue(&'static str),
    InvalidValue { flag: &'static str, value: String },
    UnexpectedPositional(String),
    AreaBoundsIncomplete,
}
```

### Resolved configuration (`crates/crpg-server/src/config.rs`)

```rust
pub(crate) struct Config {
    pub(crate) listen: std::net::SocketAddr,
    pub(crate) identity: crpg_net_quic::ServerIdentity,   // cert DER + key DER
    pub(crate) invitations: Vec<crpg_server::quic::Invitation>,
    pub(crate) save: std::path::PathBuf,
    pub(crate) identity_facts: crpg_server::save::SaveIdentity,
    pub(crate) incarnation: u64,
    pub(crate) area_bounds: Option<([crpg_core::Fx16_16; 3], [crpg_core::Fx16_16; 3])>,
    pub(crate) log_to: LogTo,
}

/// Merge the JSON file (when `--config`) under the CLI flags, load the
/// cert/key/invitation files, and validate. First failure wins.
pub(crate) fn resolve(cli: Cli) -> Result<Config, ConfigError>;

pub(crate) enum ConfigError {
    Io { path: std::path::PathBuf, source: std::io::Error },
    Json { path: std::path::PathBuf, source: serde_json::Error },
    Missing(&'static str),
    InvalidUlid(String),
    InvalidVersion(String),
    InvalidPoint(&'static str, String),
    IncarnationNotIncreasing { supplied: u64, last: u64 },
    AreaBoundsIncomplete,
}
```

The JSON file is the same keys as the flags, minus `--config`: `listen`,
`cert`, `key`, `invitations`, `save`, `campaign_id`, `campaign_version`,
`incarnation`, `area_min`, `area_max`, `log_file`. A CLI flag present on the
command line wins over the file; a value may not be `null`.

### The log writer (`crates/crpg-server/src/logline.rs`)

```rust
pub(crate) struct Log { sinks: Vec<Box<dyn std::io::Write + Send>> }
impl Log {
    pub(crate) fn new(dest: LogTo) -> std::io::Result<Log>;
    /// A test sink (a `Vec<u8>`); production uses `new`.
    pub(crate) fn from_writer(w: Box<dyn std::io::Write + Send>) -> Log;
    pub(crate) fn start(&mut self, listen: std::net::SocketAddr, incarnation: u64,
                        pin: &crpg_net_quic::CertificatePin) -> std::io::Result<()>;
    pub(crate) fn reject(&mut self, now_ms: u64, lane: u8, code: Option<u32>,
                         peer: Option<u64>, len: usize) -> std::io::Result<()>;
    pub(crate) fn stop(&mut self) -> std::io::Result<()>;
}
```

Each method writes exactly one newline-terminated UTF-8 line to every sink,
in the fixed formats below (`{}` is the value's `Display`; `pin` is
`CertificatePin::to_hex`, 64 lowercase hex):

```
crpg-server start listen={} incarnation={} pin={}
crpg-server {} reject lane={} code={} peer={} len={}
crpg-server stop
```

The `reject` line fields: `{}` (first) is `now_ms`; `lane` is `0` or `1`;
`code` is the refusal's numeric code, or `-` when the refusal carries none;
`peer` is the raw `PeerHandle` number, or `-` when unbound; `len` is the
refused frame's byte length. A failed `reject` write is ignored (the sink is
best-effort after startup); a failed `start` write fails startup.

### The library addition (`crates/crpg-server/src/quic.rs`)

```rust
impl QuicHost {
    /// The report of the most recent `pump` or `flush`, or `None` before any
    /// (Reading 8). Set at the end of `pump` and `flush`, after the report is
    /// built. The loop's `Hosting::intake` drops the returned report, so the
    /// dedicated binary reads rejections from here.
    pub fn last_report(&self) -> Option<&QuicPumpReport>;
}
```

## Behaviour

Normative. Each item names the tests that prove it. **First failure wins** in
every ordered list. "Startup fails" means: the message is written to stderr
and the process exits with code 2 (usage/config/load errors) without binding
a socket or starting the loop; only a caught server-loop error uses exit 1.

B-1. **`--help` / `--version`.** `parse` returns `CliAction::Help` /
`CliAction::Version`; `main` prints the usage text / `VERSION` to stdout and
exits 0. Neither reads a file nor binds a socket. *Tests:* D1.

B-2. **Parse errors.** An unknown flag, a flag missing its value, a
malformed value (`--listen` not `IP:port`, `--incarnation` not decimal,
`--area-min` not `x,y,z`), a stray positional argument, or one of
`--area-min`/`--area-max` without the other returns the named `CliError`;
`main` prints it to stderr and exits 2. *Tests:* D2.

B-3. **Resolution and precedence.** `resolve` reads the `--config` JSON, then
overlays every CLI flag that is `Some`; a flag present on the command line
wins over the file. A required field absent from both (`listen`, `cert`,
`key`, `invitations`, `save`, `campaign_id`, `campaign_version`,
`incarnation`) is `ConfigError::Missing(field)`. *Tests:* D3.

B-4. **File loading.** `cert`/`key` are read as raw bytes and become
`ServerIdentity::new(cert, key)` (`crates/crpg-net-quic/src/identity.rs:83`);
a missing or unreadable file is `ConfigError::Io`. `invitations` is a JSON
array (open question Q4) parsed into `Vec<Invitation>`; `save` is a path, not
read here. *Tests:* D4.

B-5. **Incarnation guard.** Before `load_host_file`, `resolve` reads
`<save>.incarnation` (`0` when absent); a supplied `incarnation` that does not
exceed it is `ConfigError::IncarnationNotIncreasing`. After a successful
load the sidecar is rewritten to the supplied value. *Tests:* D5.

B-6. **Startup order.** In order: `resolve` (B-3..B-5); `load_host_file(save,
&identity_facts, incarnation, now_ms)` (`crates/crpg-server/src/save.rs:287`);
if `area_bounds` is `Some((min, max))`, `Host::bind_area_bounds(min, max)`
(`crates/crpg-server/src/host.rs:4611`) — before unpausing, before the loop
(Reading 10); register every invitation (`QuicHost::register_invitation`);
`QuicServer::bind_resumable(ServerConfig { bind: listen, identity, limits:
ServerLimits::v1() })` then `QuicHost::start_resumable(host, server, now_ms)`
(`crates/crpg-server/src/quic.rs:578`); wrap in `QuicHosting::new`;
`runner::spawn(HostLoop::new(QuicHosting::new(quic), now_ms),
SystemClock::starting_at(now_ms))` (the handoff,
`task/T154-c1:tasks/T154.md:248`). `now_ms` is one value read once at
startup (the clock origin must equal the loop start, T154 Reading 13).
*Tests:* D6.

B-7. **Navigation wiring.** With `area_bounds`, `bind_area_bounds` is called
once, before unpause and before the loop; a `NavBake` refusal is a startup
failure (exit 2). Without `area_bounds` the host stays unbound
(`nav_revision() == None`, `crates/crpg-server/src/host.rs:4625`) and this is
not an error. Any mover the server itself spawns or transfers after binding
is `place`d by it (Reading 10); the MVP smoke spawns none. *Tests:* D7.

B-8. **The AI hook drains the report.** The `QuicAiHook` closure installed by
`QuicHosting::set_ai` runs once per cycle, after intake (cycle order: intake,
AI hook, advance, deliver; `tasks/T153.md` Reading 1). It reads
`quic.last_report()` and writes one `reject` line per refused lane-0 or
lane-1 frame, in the report's order, then does nothing else (no AI drive in
the MVP, Reading 9). *Tests:* D8, D9.

B-9. **The `reject` line.** Exactly the format of "Interface", one line per
refused frame: `now_ms` is the cycle's injected time, `lane` is `0` or `1`,
`code` is the refusal's numeric code (`-` when none), `peer` is the raw
handle or `-`, `len` is the frame length. A `reject` write that fails is
dropped, never retried, never a panic. *Tests:* D9.

B-10. **The `start` line.** Written once, after the loop thread is spawned
and running, with the bound `listen`, the `incarnation` and the
`CertificatePin` of `identity` (the operator gives this to clients out of
band, ADR-0025 `docs/adr/0025-lane0-quic-mapping-and-pinned-trust.md:53–55`).
A failed `start` write fails startup. *Tests:* D6, D10.

B-11. **Shutdown.** On stdin EOF or the line `quit`, the main thread calls
`RunnerHandle::stop()` then `join()`, writes a final `save_host_file` when
`--save` is set, writes the `stop` line, and exits 0. Any earlier error
(`RunnerGone`, `RunnerPanicked`, a step error from `RunnerExit`) is written
to stderr and exits 1. *Tests:* D11.

B-12. **Log destination.** `Log::new(Stderr)` writes to `std::io::stderr`;
`File(p)` and `Both(p)` also append to the file at `p` (created if absent).
The same line is written to every sink, in order. *Tests:* D10, D12.

B-13. **No panic on rejection.** A malformed or hostile client message is
rejected cleanly by the host (`docs/CRPG_ENGINE_SPEC.md:1488`) and yields a
`reject` line; the server process never panics. *Tests:* D13 (a scripted
client sends an oversized frame; the server logs `reject` and keeps serving).

B-14. **`QuicHost::last_report`.** `None` before any `pump`/`flush`; after
one, `Some` the exact report that call returned, unchanged by later reads.
It adds no checkpoint/save bytes (host state only) and no trace entry.
*Tests:* D8 (unit), and every existing `host_quic` suite must pass unchanged.

### T040b — the gate-12 artifact jobs

C-1. **The `server-artifacts` job.** A new job in `.github/workflows/ci.yml`
with `strategy.matrix.os = [ubuntu-latest, windows-latest]`,
`runs-on: ${{ matrix.os }}`, on GitHub-hosted runners (S001,
`.github/workflows/ci.yml:24–30`). It runs `cargo build --release --locked -p
crpg-server --bin crpg-server`, then checks the binary exists
(`target/release/crpg-server.exe` on Windows, `target/release/crpg-server`
on Linux), and uploads it with `actions/upload-artifact@v7` named
`crpg-server-linux-x86_64` / `crpg-server-windows-x86_64`,
`if-no-files-found: error`, `retention-days: 14`. *Tests:* C1 (workflow
review).

C-2. **Gate-12 server half.** The job is the server half of gate 12
(`docs/CRPG_ENGINE_SPEC.md:1333`); the client/editor half is T047c's
(`.github/workflows/ci.yml:351–357`). The Linux job builds the Godot-free
dedicated server: `cargo build -p crpg-server` must not name any godot
package (BR-21 already pins the default-feature graph,
`.github/workflows/ci.yml:154–174`). No other workflow file is added, and no
existing job changes. *Tests:* C2 (workflow review).

C-3. **Docs-only PRs still skip the Rust gates.** The new job runs on the
same `steps.scope.outputs.rust` guard as `check`/`client`, so a
documentation-only pull request reports it without building (the job always
runs and reports, `.github/workflows/ci.yml:112–147`). *Tests:* C3 (workflow
review).

## Errors

No new `HostError` or `QuicHostError` variant, no new `Display` text. The new
error types are binary-only. Exit codes: `0` clean (`--help`, `--version`,
shutdown); `2` any startup failure (parse, config, file, incarnation, bake);
`1` a caught loop-thread error (`RunnerGone`/`RunnerPanicked`/`RunnerExit`).

| Error | When |
|---|---|
| `CliError::UnknownFlag(flag)` | an argument starts `--`/`-` and is not a known flag |
| `CliError::MissingValue(flag)` | a value flag is last, or followed by another flag |
| `CliError::InvalidValue { flag, value }` | a value fails its parse (`--listen`, `--incarnation`, `--area-min`, `--area-max`) |
| `CliError::UnexpectedPositional(arg)` | a non-flag argument is present |
| `CliError::AreaBoundsIncomplete` | exactly one of `--area-min`/`--area-max` is given |
| `ConfigError::Io { path, source }` | cert/key/log/invitation file missing or unreadable |
| `ConfigError::Json { path, source }` | the `--config` file is not valid JSON, or a value has the wrong type |
| `ConfigError::Missing(field)` | a required field is absent from CLI and file |
| `ConfigError::InvalidUlid(text)` | `campaign_id` is not a 26-character ULID text |
| `ConfigError::InvalidVersion(text)` | `campaign_version` is outside the save alphabet (`crates/crpg-server/src/save.rs:92`) |
| `ConfigError::InvalidPoint(flag, text)` | an `--area-min`/`--area-max` value is not exactly three integers |
| `ConfigError::IncarnationNotIncreasing { supplied, last }` | `supplied <= last` in `<save>.incarnation` |
| `ConfigError::AreaBoundsIncomplete` | the CLI and file together give exactly one bound |

Every startup error prints exactly one line to stderr —
`crpg-server: <variant>[: <detail>]` — then the process exits 2.

## Tests

Unit cases in `crates/crpg-server/tests/dedicated_binary.rs` for the pure
`parse`/`resolve`/`Log` parts, plus black-box cases driving
`env!("CARGO_BIN_EXE_crpg-server")` for the process-level behaviours (the
`crpgc` discipline, `crates/crpg-cli/tests/demo.rs:3–9`). The serve path is
proved by B-6/D6 against the existing host save fixture and a checked-in
test-only DER cert/key (like the QUIC tests' DER fixture,
`crates/crpg-net-quic/AGENTS.md` "test-only" trap), never an operator
identity.

- **D1 `help_and_version`.** `parse` of `["--help"]` → `Help`, of
  `["--version"]` → `Version`; the binary's `--version` prints `0.1.0\n` to
  stdout and exits 0 (the package version,
  `crates/crpg-server/Cargo.toml:3`), and `--help` prints text containing
  `Usage:` and `--listen` and exits 0. *Control:* `["--version", "--listen"]`
  still prints the version and exits 0 before parsing the rest. *Fails if:*
  `--help`/`--version` read a file, bind a socket, or exit non-zero.
- **D2 `parse_errors`.** Each of `["--bogus"]`, `["--listen"]` (missing
  value), `["--incarnation", "x"]`, `["--area-min", "0,0"]` (not 3 ints),
  `["stray"]`, `["--area-min", "0,0,0"]` (no `--area-max`) returns the named
  `CliError`; the binary prints `crpg-server: <Variant>` to stderr and exits
  2. *Control:* `["--area-min", "0,0,0", "--area-max", "1,1,1"]` parses
  cleanly. *Fails if:* any of these exits 0, or a malformed value is silently
  accepted.
- **D3 `precedence_and_missing`.** A config file with `listen` set and the
  CLI giving `--listen` uses the CLI value; a file value with no CLI flag is
  used; a required field absent from both is `Missing("<field>")`. *Control:*
  the same file with the CLI flag omitted takes the file's `listen`. *Fails
  if:* a CLI `None` overwrites a file value, or a present-but-empty value is
  not `Missing`.
- **D4 `file_loading`.** A missing `--cert` path → `ConfigError::Io`; a valid
  DER cert/key pair loads into `ServerIdentity::new(cert, key)`. *Control:*
  the same bytes read from a real temp file succeed. *Fails if:* a cert/key
  that `bind` would reject loads without error, or a valid pair is refused.
- **D5 `incarnation_guard`.** With `<save>.incarnation` containing `7`,
  `--incarnation 7` → `IncarnationNotIncreasing { supplied: 7, last: 7 }`;
  `--incarnation 8` → `Ok` and the sidecar is rewritten to `8`; a missing
  sidecar with `--incarnation 1` → `Ok`. *Control:* `--incarnation 6` also
  refuses (not just equality). *Fails if:* the guard allows a value ≤ `last`,
  or rewrites the sidecar on a refused load.
- **D6 `startup_order_and_start_line`.** With a host save, a test cert/key,
  an invitation file and `--area-min/--area-max`, the binary binds, spawns
  and writes exactly one `start` line whose `pin` is the 64-hex
  `CertificatePin` of the cert, then serves. *Control:* without
  `--area-min/--area-max` the binary still starts (B-7) and no `bind_area_bounds`
  runs. *Fails if:* `bind_area_bounds` runs after unpause or after the loop
  starts, the `start` line is written before spawn, or a missing save/cert
  binds a socket.
- **D7 `navigation_wiring`.** With bounds present, `bind_area_bounds(min,max)`
  is called exactly once before unpause (a host whose `nav_revision()` was
  `None` now returns `Some`); a `NavBake` refusal fails startup with exit 2.
  *Control:* with no bounds, `nav_revision()` stays `None` and startup
  succeeds. *Fails if:* the order is wrong (bound after unpause), or no-bounds
  is treated as an error.
- **D8 `last_report`.** `QuicHost::last_report()` is `None` before any pump;
  after one `pump`, it is `Some` the report that call returned; after a
  second pump it is the newer report. *Control:* a host that never pumps
  stays `None` across `last_report` calls. *Fails if:* the field is saved in
  a checkpoint (its bytes change), or `last_report` mutates the host.
- **D9 `reject_line`.** `Log::reject(1000, 0, Some(30), Some(1), 4097)` writes
  `crpg-server 1000 reject lane=0 code=30 peer=1 len=4097\n`;
  `Log::reject(1000, 0, None, None, 0)` writes `crpg-server 1000 reject
  lane=0 code=- peer=- len=0\n`. *Control:* `lane=1` changes `lane=0` to
  `lane=1` and nothing else. *Fails if:* the line is not byte-exact, a field
  is reordered, or a refused frame produces no line (B-13).
- **D10 `log_destination`.** `Log::new(Stderr)` writes only to stderr;
  `Log::new(Both(p))` writes the same `start`/`reject`/`stop` lines to both
  stderr and the file at `p`. *Control:* `Log::from_writer(Vec<u8>)` captures
  the exact bytes (used by D9). *Fails if:* a sink is skipped or the file is
  truncated rather than appended.
- **D11 `shutdown`.** After stdin EOF (or the line `quit`), the binary calls
  `stop`/`join`, writes a final `save_host_file` when `--save` is set, writes
  `crpg-server stop\n`, and exits 0. *Control:* a `quit` line and EOF produce
  the same exit. *Fails if:* the loop thread is leaked, the final save is
  skipped when `--save` is set, or exit is non-zero on a clean shutdown.
- **D12 `log_line_order`.** A `start` line precedes every `reject` line, which
  precedes the `stop` line, in the shared sink. *Control:* with no clients,
  only `start` and `stop` appear. *Fails if:* a `reject` line precedes
  `start`.
- **D13 `no_panic_on_rejection`.** A scripted client sends an oversized
  lane-0 frame; the server writes one `reject` line for it and keeps serving
  (the next valid frame is accepted), and the process does not panic.
  *Control:* the same client sending a valid frame produces no `reject` line.
  *Fails if:* the server panics, drops the connection without a line, or a
  `reject` write error propagates.
- **C1–C3 (workflow review, not code).** A reviewer reads `.github/workflows/ci.yml`
  and confirms the `server-artifacts` job: the two-OS matrix on GitHub-hosted
  runners, `cargo build --release --locked -p crpg-server --bin crpg-server`,
  the two artifact names and `if-no-files-found: error`, the `rust` scope
  guard, and that no existing job or other workflow file changed.

## What must not change

- Goldens, byte for byte (none added, none moved): every
  `crates/crpg-sim/tests/goldens/*`, `crates/crpg-testkit/goldens/*`, and any
  other `*.golden`/`.txt` under `crates/*/tests/goldens`.
- Every existing `crpg-server` test file, byte for byte and passing:
  `host_capture.rs`, `host_save.rs`, `host_quic.rs`, `host_quic_transfer.rs`,
  `host_legal.rs`, `host_nav.rs`, `host_ops.rs`, `host_pause.rs`,
  `host_records.rs`, `host_records_contract.rs`, `host_transfer.rs`,
  `host_v3_publish.rs`, `host_v3_session.rs`, `host_v3_timing.rs`, `support/`,
  `fixtures/`, and every `#[cfg(test)]` case in `host.rs`, `checkpoint.rs`,
  `capture.rs`, `nav.rs`, `save.rs`, `quic.rs`, `run.rs`, `runner.rs`.
- Frozen library sources: `src/{host,capture,checkpoint,nav,host_v3,save,lib}.rs`
  and every line of `src/run.rs`, `src/runner.rs` and `src/quic.rs` except
  the one `last_report` item (Reading 8). `main.rs` is replaced, not frozen.
- Every public library shape: `HostError`, `QuicHostError`, `SaveError` and
  every `Display` text; `AdvanceReport`, `AdmissionResult`, `DeliveryKind`,
  `HostOp`, `HostRecord`, `NavBinding`, `HostSettings`, `ControlGrant`,
  `QuicPumpReport`, `HostCall`, `Lane1Call`; checkpoint versions 1 and 2 and
  their bytes; `World` and tick order.
- `Cargo.toml` (no new `[dependencies]`/`[dev-dependencies]`), `Cargo.lock`,
  `deny.toml`, `rust-toolchain.toml`, `crpg-contracts`, `tools/`. Every other
  crate, `.github/workflows/ci.yml` except the new `server-artifacts` job,
  and the determinism lint.

## Dependency edges

No new edge. The binary uses `std` plus what the crate already has:
`serde_json` (`crates/crpg-server/Cargo.toml:32–33`) for the config and
invitation files, `crpg-net-quic` for `ServerIdentity`/`CertificatePin`/
`Credential`/`ServerConfig` (already an edge, `:27`), `crpg-core` for
`Fx16_16`/`Ulid`/`EntityId`, and the library's own `quic`/`save`/`run`/
`runner` modules. No `[dependencies]`, `[dev-dependencies]`,
`[build-dependencies]` or `[target.*]` table changes, so `tools/lint/deps.py`
sees the same graph and `ALLOWED` is untouched (`tools/lint/deps.py:46–72`).

The no-new-dependency alternative is the point of Readings 2 and 4: where an
argument parser or a logging crate would be used, `parse` is hand-rolled over
`std::env::args` and `Log` is a `&mut dyn io::Write` writer. If the architect
prefers clap or `tracing`, that is a new external dependency needing the
user's H-dep approval (`AGENTS.md` "Do not add dependencies without being
asked"); this draft does not add one.

## Definition of done

On the T040 branch (cut after T154d merges), all green:

```text
cargo fmt --all
cargo clippy -p crpg-server --all-targets --locked -- -D warnings
cargo test -p crpg-server --locked
cargo build --release --locked -p crpg-server --bin crpg-server
python tools/lint/deps.py
python tools/lint/determinism.py
python -m unittest discover -s tools/lint -p "test_*.py"
cargo deny check
git diff --check
```

`git diff --name-only <base>` lists only the owned files (Parts). Docs, with
a signed agent-log entry in each: `docs/architecture/crpg-server.md` (the
"planned and **not** here" list shrinks; a new "Dedicated binary (T040)"
paragraph), `crates/crpg-server/AGENTS.md` (invariant 7 amended for the
binary, the new surface and traps). T040b is proven by workflow review (C1–C3)
and by the gate-12 server build actually producing both artifacts.

## Out of scope

- Any code change in this draft slot: the file is a contract, not an
  implementation.
- The `--headless-deterministic` mode (`docs/CRPG_ENGINE_SPEC.md:958`):
  replay-file inputs, fixed seed, state hash per N ticks. Not wired here.
- OS service integration (systemd/Windows Service), listen-server mixed mode,
  and the GM/admin endpoint (`docs/architecture/crpg-server.md:30–34`).
- Campaign/area data loading: with no `crpg-data` edge the server serves a
  saved world and operator-supplied identity and bounds; authoring the
  campaign from data is the campaign-load task (T089/T041), not T040.
- The AI hook's `drive_ai` wiring (needs `Sides` from authored data, Reading 9).
- T041's scripted headless clients and the gate-13 smokes; T042's
  malicious-client suite; T089's public dedicated-server release builds.
- Log rotation, log levels, and any log/metrics crate.
- The client and editor artifact jobs (T047c) and any product build beyond
  the dedicated server.

## Open questions

Each is a product or shape choice this draft could not settle from the
binding inputs; each carries a recommendation.

1. **Signal handling (Ctrl-C / SIGINT / SIGTERM).** A std-only server cannot
   install a Windows console-control handler without `unsafe` (forbidden
   outside `crpg-godot`, `AGENTS.md:25–27`), nor a Unix signal handler without
   a dependency or `unsafe`. This draft's shutdown is stdin EOF / `quit`.
   *Recommendation:* ship the std-only console for T040; file a follow-up for
   OS signal handling (with a `ctrlc`-style dependency, H-dep) only if
   operators need it before then.
2. **Default listen address.** This draft makes `--listen` required rather
   than invent a well-known port. *Recommendation:* keep it required; if the
   user wants a default, name one now (e.g. `0.0.0.0:2106`) and it becomes a
   `parse` default plus a D3 control.
3. **Certificate/key file encoding.** `ServerIdentity::new` takes DER bytes
   (`crates/crpg-net-quic/src/identity.rs:83`), so this draft reads cert/key
   as DER. PEM needs `rustls-pemfile` (a dependency). *Recommendation:* DER
   (std-only); document that operators convert PEM → DER.
4. **Invitation file format and the grant vocabulary.** `Invitation` holds
   `credential`, `control: ControlGrant`, `grants: DisclosureGrants`
   (`crates/crpg-server/src/quic.rs:81–88`). The exact JSON encoding of the
   grant shapes is not fixed here. *Recommendation:* a JSON array of
   `{"credential": "<64 hex>"}` plus a control/disclosure shape that
   serializes the existing types; T027b/c supply production grant values, and
   the MVP smoke uses a degenerate "control and disclose every entity" shape.
   The architect must pin the exact bytes before implementation.
5. **AI on the dedicated server.** With no `crpg-data` edge the server cannot
   build `crpg_ai::Sides`. *Recommendation:* run with no `drive_ai` in the
   MVP (Reading 9); wire it in the campaign-load task.
6. **Incarnation sidecar placement.** This draft uses `<save>.incarnation`.
   *Recommendation:* keep it; if a server-state directory is preferred, name
   it and the sidecar becomes `<state-dir>/<save-stem>.incarnation`.
7. **`last_report` ownership.** T154d is contract-only, so the one additive
   item in `quic.rs` is an amendment, not a rewrite. *Recommendation:* keep it
   in T040a as specified, with the Manager-edit note below.
8. **`--area-min`/`--area-max` value units.** This draft takes three raw
   integers parsed to `Fx16_16`. *Recommendation:* raw integers (the crate's
   own units, `crates/crpg-server/src/host.rs:4613`); metres would add a
   float/scaling step to the CLI with no benefit to the smoke.

## Manager edits

## Agent log
