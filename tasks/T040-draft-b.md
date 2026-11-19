# T040 — dedicated `crpg-server` binary (blind draft b)

**Status: draft, 2026-11-19 (UTC); blind draft b (no other draft read).
Not approved: a Claude architect writes the final contract. Nothing here is
approved.**

Written from the fact sheet (`tasks/T040-facts.md`), re-checked code, and
the approved T153 contract's form (`tasks/T153.md`).

## Task

Give `crpg-server` its dedicated binary: the thin wiring shell that exists
today (`crates/crpg-server/src/main.rs:9-12` prints "dedicated adapter not yet
implemented" and exits 1) becomes the runnable dedicated server for Windows
and a Godot-free Linux build. It loads a host save, binds the navigation
mesh, starts the T154 run loop on the resumable QUIC adapter, serves players
until shutdown, and logs one server line per rejected client message. It
adds the CI artifact jobs for both dedicated-server binaries (gate 12).
Gate 13 smokes, the fixture-campaign breadth of gate 10, and campaign loading
stay out (T041's). D66b leaves this program to T040
(`tasks/DECISIONS-D66.md:18-25`); T154 hands it
`runner::spawn(HostLoop::new(QuicHosting::new(quic), now),
SystemClock::starting_at(now))` (`tasks/T154.md:245-252`).

## Parts

| Part | Crate | Scope | Size (source + tests) | S-set | Review |
|---|---|---|---|---|---|
| T040a | `crpg-server` | `src/main.rs`: CLI, config, logging, startup order, shutdown; black-box `server_binary` suite | ~220 + ~330 | **yes** (`docs/IMPLEMENTATION_PLAN.md:245`: "C → I, S") | Claude reviewer mutant run on `src/main.rs`, one fix round |
| T040b | CI | gate-12 artifact jobs in `.github/workflows/ci.yml` | ~60 YAML + job green on both OS legs | **yes** (`docs/IMPLEMENTATION_PLAN.md:86-98`: workflows are serialized) | none (config only) |

Order: T040a, then T040b (the jobs build T040a's binary). Either may start
from the contract, but only one lands at a time.

### T040a owns
- `crates/crpg-server/src/main.rs` (the full wiring: args, config, startup,
  log lines, shutdown; `#![forbid(unsafe_code)]` and `#![warn(missing_docs)]`
  stay, `crates/crpg-server/src/main.rs:1-2`)
- `crates/crpg-server/tests/server_binary.rs` (implementer, new; black-box
  over `env!("CARGO_BIN_EXE_crpg-server")`, the `crpg-cli` discipline,
  `crates/crpg-cli/tests/demo.rs:3-9`)
- `docs/architecture/crpg-server.md` (a new section "The dedicated binary
  (T040)"), `crates/crpg-server/AGENTS.md` (binary surface, log-line trap,
  startup-order trap)
- goldens: none new, none moved.

### T040b owns
- `.github/workflows/ci.yml` (two additive jobs, `dedicated-windows` and
  `dedicated-linux`; no existing job edited)
- goldens: none new, none moved.

## Readings

Design decisions this contract makes. Each needs approval with the contract.

1. **The binary stays a wiring shell; policy stays in the library.**
   `src/main.rs` is a wiring shell with no host policy
   (`crates/crpg-server/AGENTS.md:24-26`); the library is the one
   platform-neutral host the embedded and dedicated adapters reuse
   (`docs/architecture/crpg-server.md:5-9`). T040 wires certificate, config,
   save, mesh, adapter, loop and shutdown, and adds no `Host` method, no
   quic method, no ledger. *Rejected:* host policy in `main.rs` (a second
   authority beside the library D02 forbids).
2. **One platform-neutral host, three adapters.** Windows single-player
   embeds the same implementation in-process
   (`docs/CRPG_ENGINE_SPEC.md:155-156`); Windows and Linux dedicated
   processes wrap that same implementation (`docs/CRPG_ENGINE_SPEC.md:157`);
   the shape is one implementation with in-process and dedicated adapters
   (`docs/CRPG_ENGINE_SPEC.md:915-917`). The dedicated binary hosts
   `QuicHosting` (T154d, `tasks/T154.md:403-417`); single-player keeps
   `LocalHost` (`crates/crpg-server/src/run.rs:332`). *Rejected:* a
   dedicated-only host type (reimplements authority per platform).
3. **The loop is T154's, unchanged.** The cycle is intake, AI stage,
   advance, deliver, retire captures (`tasks/T154.md:123-135`); nothing in
   the loop reads pause (`crates/crpg-server/AGENTS.md:467`); `run.rs`
   never blocks (`crates/crpg-server/AGENTS.md:468-471`). T040 calls
   `HostLoop::new`, `spawn`, `RunnerHandle::call/stop/join` and never
   reorders stages. *Rejected:* a second pump as the deliver stage
   (`tasks/T154.md:237-242` already rejects it: a second pump admits after
   the tick).
4. **The resume route is `bind_resumable` plus `start_resumable`.**
   `QuicHost::start` takes a server from `bind`/`bind_dual` only and polls
   only `poll_event` (`crates/crpg-server/src/quic.rs:560-565`), while
   `start_resumable` accepts any `bind*` including `bind_resumable`
   (`crates/crpg-server/src/quic.rs:574-584`) and reports the route with
   `is_resumable` (`crates/crpg-server/src/quic.rs:586-589`). The filed
   manager edit says the dedicated server binds `bind_resumable` and calls
   `start_resumable` (`tasks/T025b.md:862-863`, cited in the fact sheet).
   *Rejected:* `start` (resume notices stop its poller for good).
5. **Movement wiring is the embedding's job: `bind_area_bounds` at area
   load, `place` after transfers and spawns.** `Host::bind_area_bounds`
   bakes an area's two bounds into a mesh and binds it
   (`crates/crpg-server/src/host.rs:4606-4621`); `Host::place` is the
   trusted placement of one entity
   (`crates/crpg-server/src/host.rs:4629-4645`); later-created movers are
   the caller's to `place` (T037 Reading 5, `tasks/T037.md:160-165` in the
   fact sheet). The backlog row files this at T040
   (`tasks/BACKLOG.md:344`). *Rejected:* binding once at boot (a restored
   host has `nav_revision() == None` until it rebinds,
   `docs/architecture/crpg-server.md:377-380`).
6. **Startup order is load, incarnation, bind, unpause, hook, serve.** A
   save loads through `load_host_file` into an unbound `Restore`-paused
   host under a greater incarnation (`tasks/T039.md:124-130`); the mesh is
   not saved so a restored host must bind it again
   (`docs/architecture/crpg-server.md:377-380`); pause and settings are
   never persisted so the operator calls `unpause` and `set_settings` again
   (`crates/crpg-server/AGENTS.md:246-250`). `QuicHost::start` refuses a
   closed host, a host in use, or a time regression
   (`crates/crpg-server/src/quic.rs:557-572`). *Rejected:* serving a
   `Restore`-paused load (it would tick nothing and confuse every client).
7. **No new dependency: std-only args and logging.** An argument parser and
   a logger both need the user's approval (`AGENTS.md:52`; D67a chose (a)
   over adding a logging library, `tasks/DECISIONS-D67.md:5-12`).
   `crpg-cli` already retains hand-rolled per-command parsers with no
   library swap (the `crpg-cli` contract, invariants 3–4). So `main.rs`
   parses `std::env::args_os` by hand and writes log lines with `std`
   only. *Rejected:* `clap`/`argh`, `log`/`tracing`/`env_logger` (new
   external crates for what `std` covers).
8. **The rejected-message line belongs to the server program (D67a).**
   Every malicious case must produce "a clean rejection and a log line,
   never a panic" (`docs/CRPG_ENGINE_SPEC.md:1488`); actually writing the
   line is T040's or T041's job (`tasks/DECISIONS-D67.md:9`) with a
   where-do-logs-go decision in the backlog row (`tasks/BACKLOG.md:341`).
   This contract puts it in T040a: the adapter's refusals already carry
   exact texts, and the running server is what sees them. *Rejected:*
   leaving it to T041 alone (T041 tests clients, not the product log).
9. **Logs go to stderr, and to a file when configured.** No server crate
   writes a log anywhere today (D67a,
   `tasks/DECISIONS-D67.md:7`). Stderr keeps the black-box discipline
   (exit codes plus byte assertions, `crates/crpg-cli/tests/demo.rs:3-9`)
   and a `--log-file` path gives operators persistence without a logging
   framework. *Rejected:* stdout (stdout stays machine-clean for the
   deterministic mode's hashes); a new log framework (Reading 7).
10. **Gate 12 is two additive artifact jobs; gate 13 smokes stay T041's.**
    Gate 12 builds Windows client + editor + server and Linux headless
    server artifacts (`docs/CRPG_ENGINE_SPEC.md:1333`); gate 13's smokes
    are scripted clients completing the fixture campaign
    (`docs/CRPG_ENGINE_SPEC.md:1334-1335`); the only artifact upload today
    is the Windows client slice
    (`.github/workflows/ci.yml:351-357`) and no dedicated-server job
    exists (fact sheet §CI). The M4 exit criteria name both
    (`docs/IMPLEMENTATION_PLAN.md:179-181`). *Rejected:* editing the
    `check`/`client` jobs (additive jobs keep their gates intact); folding
    gate 13 into T040 (T041 owns scripted headless clients,
    `docs/IMPLEMENTATION_PLAN.md:248`).

## Interface

All new code lives in `crates/crpg-server/src/main.rs` (the package-root
binary, `crates/crpg-server/Cargo.toml:12-14`; no `src/bin/*.rs` exists,
fact sheet §"Existing binaries"). The `[lib]` target (`crates/crpg-server/Cargo.toml:9-10`),
`Cargo.lock`, `deny.toml`, workflows and every other crate are untouched by
T040a. Every item below is in `main.rs`, `std` only, with a doc comment
(`crates/crpg-server/src/main.rs:1-2` keep both crate-root attributes).

```rust
/// Exit buckets, the `crpgc` convention (exit 0 verified, 1 domain, 2 usage).
const EXIT_OK: i32 = 0;
const EXIT_RUNTIME: i32 = 1;
const EXIT_USAGE: i32 = 2;

/// Where one rejected client message is written (Reading 9).
#[derive(Debug, Clone, PartialEq, Eq)]
enum LogSink {
    /// Standard error only.
    Stderr,
    /// Standard error and an appended file.
    File(std::path::PathBuf),
}

/// Operator configuration after CLI-over-file precedence (Reading B-2).
#[derive(Debug, Clone, PartialEq, Eq)]
struct ServerConfig {
    /// Save file to load with `load_host_file` (`tasks/T039.md:218-223`).
    save_path: std::path::PathBuf,
    /// Campaign identity the save must match (`tasks/T039.md:183-191`).
    campaign_id: crpg_core::Ulid,
    /// Expected campaign version text, 1..=64 bytes of the version alphabet.
    campaign_version: String,
    /// Incarnation for the load: greater than every incarnation ever used
    /// for that authority (`tasks/T039.md:218-223`; caller fact,
    /// `crates/crpg-server/AGENTS.md:302`).
    incarnation: u64,
    /// Address the QUIC server binds (default in Behaviour).
    bind_addr: std::net::SocketAddr,
    /// QUIC server from `bind_resumable` (manager edit,
    /// `tasks/T025b.md:862-863` in the fact sheet).
    invitation_path: std::path::PathBuf,
    /// Certificate chain file (rotation means a new invitation;
    /// `docs/adr/0025-lane0-quic-mapping-and-pinned-trust.md:56-59`).
    cert_path: std::path::PathBuf,
    /// Private key file.
    key_path: std::path::PathBuf,
    /// Area bounds file for `bind_area_bounds`
    /// (`crates/crpg-server/src/host.rs:4611-4621`).
    area_path: std::path::PathBuf,
    /// Where server logs go (Reading 9).
    log: LogSink,
    /// `--headless-deterministic` mode (`docs/CRPG_ENGINE_SPEC.md:958`).
    deterministic: Option<DeterministicConfig>,
}

/// Inputs for `--headless-deterministic` mode.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DeterministicConfig {
    /// Replay file of inputs (no network in this mode).
    replay_path: std::path::PathBuf,
    /// Fixed seed.
    seed: u64,
    /// Print one state hash per this many ticks.
    hash_every_n_ticks: std::num::NonZeroU64,
}

/// One CLI usage failure: exit 2 with exactly one stderr line.
#[derive(Debug, Clone, PartialEq, Eq)]
struct UsageError(String);

/// Parses `std::env::args_os` by hand (Reading 7); never panics on input.
fn parse_args(args: &[std::ffi::OsString]) -> Result<ServerConfig, UsageError>;

/// Loads the authority: `load_host_file` then `bind_area_bounds` at area
/// load, returning the host and the `now_ms` the loop starts at (Reading 6).
fn load_authority(cfg: &ServerConfig) -> Result<(crpg_server::host::Host, u64), String>;

/// Serves until shutdown: `start_resumable`, `QuicHosting`, `spawn` with
/// `SystemClock::starting_at(now)` (`tasks/T154.md:245-252`), `join`.
fn serve(cfg: &ServerConfig) -> i32;

/// Writes one server log line for a rejected client message (D67a,
/// `tasks/DECISIONS-D67.md:9`); never prints a credential
/// (`crates/crpg-server/AGENTS.md:370-372`).
fn log_rejection(sink: &LogSink, line: &str);

fn main() -> !;   // parses, serves, exits with EXIT_OK / EXIT_RUNTIME / EXIT_USAGE
```

CLI grammar (exact): `crpg-server [--config <file>] [--save <path>]
[--incarnation <u64>] [--bind <addr:port>] [--invitation <path>]
[--cert <path>] [--key <path>] [--area <path>] [--log-file <path>]
[--headless-deterministic <replay> --seed <u64> --hash-every <n>]
[--help]`. `--help` prints usage to stdout and exits 0.

## Behaviour

Normative. Each item names the tests that prove it. **First failure wins**
in every ordered list. `now` is the `now_ms` returned by `load_authority`.

B-1. **Exit buckets.** `--help` (alone or with anything) prints usage to
stdout and exits `EXIT_OK`. Any usage failure (unknown flag, missing or
unparsable value, missing required input, `--hash-every 0`) prints exactly
one stderr line starting with `crpg-server: ` and exits `EXIT_USAGE` with
empty stdout. Any runtime failure (unreadable save, identity mismatch,
bind/start refusal) prints its line(s) to stderr and exits `EXIT_RUNTIME`.
Clean shutdown exits `EXIT_OK`. *Tests:* S1, S2, S3.
B-2. **Config precedence.** For each field: CLI flag, then `--config` file,
then the built-in default (`bind` default `0.0.0.0:7777`;
`hash_every_n_ticks` default 100; log default `Stderr`). A missing file
named only by `--config` is a runtime failure, not usage. *Tests:* S2.
B-3. **Startup order.** On serve: (1) `load_host_file(save, identity,
incarnation, now)` (`tasks/T039.md:218-223`); (2) read the area bounds
file and call `bind_area_bounds(min, max)`
(`crates/crpg-server/src/host.rs:4611-4621`); (3) `set_settings` then
`unpause(now)` (a restored host is `Restore`-paused,
`crates/crpg-server/AGENTS.md:246-250`); (4) `set_ai` with the operator's
AI hook (T153's `drive_ai`, `tasks/T153.md:60-66`); (5) bind the QUIC
server with `bind_resumable` and `QuicHost::start_resumable(host, server,
now)` (`crates/crpg-server/src/quic.rs:578-584`); (6)
`runner::spawn(HostLoop::new(QuicHosting::new(quic), now),
SystemClock::starting_at(now))` with the clock origin equal to the loop
start (`tasks/T154.md:245-252`). First failure stops startup with
`EXIT_RUNTIME`; nothing serves half-started. *Tests:* S3, S4, S7.
B-4. **The loop served is T154d's.** Intake is `quic.pump(now)`, deliver
is `quic.flush(now)`, captures retire every cycle
(`tasks/T154.md:403-417`). The binary never reorders stages and never
reads the host except through `RunnerHandle::call`. *Tests:* S4.
B-5. **Movement wiring.** `bind_area_bounds` runs at area load (B-3 step
2); `place(entity, position)`
(`crates/crpg-server/src/host.rs:4636-4645`) runs after every area
transfer and every spawn before the entity is admitted to play. With no
mesh bound, route requests are refused, never teleported
(`docs/architecture/crpg-server.md:411-414`). *Tests:* S5.
B-6. **One log line per rejected client message.** Every client frame the
adapter refuses (any `Refused`/`Rejected` disposition and every
close-with-code from the malicious suite,
`docs/CRPG_ENGINE_SPEC.md:1488`) appends exactly one line to the sink:
`rejected <ordinal> <code> <detail>\n`, where `<ordinal>` is the delivery
ordinal (never reset in a snapshot session,
`docs/architecture/crpg-server.md:741-746`), `<code>` the refusal name,
and `<detail>` carries no credential bytes
(`crates/crpg-server/AGENTS.md:370-372`). Never a panic. *Tests:* S6.
B-7. **Where logs go.** Default sink is stderr; with `--log-file`, each
line goes to stderr and is appended to the file (created with its parent
directories if missing; an uncreatable file is a runtime failure before
serving). *Tests:* S6.
B-8. **Shutdown.** On signal or operator stop: `RunnerHandle::stop`,
then `join`; the loop thread ends with no further step
(`tasks/T154.md:541-549`); committed delivery is flushed, staged work is
refused, the transport is closed (D02 shutdown,
`docs/architecture/crpg-server.md:117-140`); an optional final
`save_host_file` runs before the close when `--save-on-shutdown` is set.
Exit `EXIT_OK` after a clean close. *Tests:* S7.
B-9. **`--headless-deterministic` mode.** With the flag: no network
(no bind, no invitation, no certificate); inputs come from the replay
file; the seed is fixed; stdout carries exactly one state hash per
`hash_every_n_ticks` ticks (`docs/CRPG_ENGINE_SPEC.md:958`). Two runs of
the same inputs print byte-identical stdout. *Tests:* S4.
B-10. **Authority boundary.** Embedded single-player and both dedicated
processes serve the same one implementation
(`docs/CRPG_ENGINE_SPEC.md:155-157`); in-memory transport preserves the
boundary and clients never mutate authoritative state directly
(`AGENTS.md:13-16`). The binary performs no gameplay call outside the
`Host`/`QuicHost` public API. *Tests:* S4 (hash equality with a
`LocalHost` twin driven through the same inputs).
B-11. **Incarnation discipline.** The load incarnation is greater than
every incarnation ever used for that authority
(`crates/crpg-server/AGENTS.md:302`); `start_resumable` on a host with
bindings or staged ingress is `HostInUse` and refuses startup
(`crates/crpg-server/src/quic.rs:591-608`). *Tests:* S3.
B-12. **CI artifacts (T040b).** `dedicated-windows` builds
`crpg-server` on `windows-latest` and uploads
`crpg-server-windows-x86_64`; `dedicated-linux` builds on
`ubuntu-latest` and uploads `crpg-server-linux-x86_64`; both run
`--locked` against the committed graph. Linux headless support never
imports Godot (`docs/CRPG_ENGINE_SPEC.md:168`): the BR-21 default-graph
query (`.github/workflows/ci.yml:150-174`) still passes. *Tests:* C1.

## Errors

| Item | Value |
|---|---|
| CLI usage failure (unknown flag, missing/unparsable value, `--hash-every 0`) | `UsageError(text)`; exactly one stderr line `crpg-server: {text}\n`; exit `EXIT_USAGE` |
| `load_host_file` refusal (`InvalidIdentity`, `CampaignMismatch`, `Persist`, `Checkpoint`, `tasks/T039.md:225-251`) | stderr line; exit `EXIT_RUNTIME`; nothing serves |
| `bind_area_bounds` refusal (`NavBake`, `tasks/T037.md:459-465` in the arch doc) | stderr line; exit `EXIT_RUNTIME`; nothing serves |
| `start_resumable` refusal (`HostClosed`, `HostInUse`, `TimeRegression`, `crates/crpg-server/src/quic.rs:591-608`) | both returned unchanged; stderr line; exit `EXIT_RUNTIME` |
| `spawn` I/O failure (`std::io::Error`) | stderr line; exit `EXIT_RUNTIME` |
| runner ends with `LoopError::TimeRegression` (clock origin before loop start, `tasks/T154.md:532-539`) | startup bug: exit `EXIT_RUNTIME` with the `TimeRegression at run/step` text (`crates/crpg-server/src/run.rs:90-96`) |
| client message refused | `log_rejection` line (B-6); never an exit, never a panic |

No new `HostError`, `QuicHostError`, `SaveError` or `LoopError` variant. No
credential bytes in any error, report or log line
(`crates/crpg-server/AGENTS.md:370-372`).

## Tests

Black-box over `env!("CARGO_BIN_EXE_crpg-server")` as a real process
(`crates/crpg-cli/tests/demo.rs:3-9`): exit codes plus byte assertions,
never binary internals. Every case has a control that changes one input
and flips the expectation, and a "fails if". Ports use `127.0.0.1:0`
ephemeral binds where the OS assigns them; no case asserts a wall-clock
duration.

- **S1 `help_and_usage_buckets`.** `--help` → exit 0, usage on stdout,
  empty stderr. `--bogus` → exit 2, empty stdout, exactly one stderr line
  starting with `crpg-server: `. No args → exit 2 with the same shape
  (every field required except `--bind`, `--log-file`, `--config`
  defaults). *Control:* `--help --bogus` still exits 0. *Fails if:* usage
  prints two lines or stdout is non-empty on a usage failure.
- **S2 `config_precedence`.** A `--config` file naming a save plus CLI
  `--bind 127.0.0.1:7788` starts (or fails at load, never at parse) while
  the same file with CLI `--hash-every 0` exits 2 before touching the
  filesystem. *Control:* drop the CLI flag → the file's value rules.
  *Fails if:* the file beats the flag.
- **S3 `bad_save_refuses_startup`.** Nonexistent `--save` → exit 1 with a
  stderr line and nothing listening. A save copied under the wrong
  `campaign_id` → exit 1 `CampaignMismatch` (`tasks/T039.md:225-251`).
  A save reloaded with an incarnation `<=` the saving run's →
  `Checkpoint(EpochReused)`, exit 1. *Control:* the same save with the
  right identity and a greater incarnation proceeds past load (it may stop
  at bind for lack of a port, but never with a load text). *Fails if:* any
  refusal serves half-started.
- **S4 `deterministic_hashes_match`.** `--headless-deterministic <replay>
  --seed 7 --hash-every 10` over the `combat_basic` fixture campaign
  prints exactly `ceil(ticks/10)` hash lines; two runs are byte-identical;
  the final hash equals a `LocalHost` twin driven through the same inputs
  (`authority_hash() == history_hash(&mirror)` pattern,
  `tasks/T153.md:466-474`). *Control:* `--hash-every 5` prints twice as
  many lines with the same final hash. *Fails if:* any network bind is
  attempted (no socket opens) or the two runs differ.
- **S5 `area_load_binds_and_spawns_place`.** A fixture area boots the
  server; `bind_area_bounds` ran (a trusted read shows
  `nav_revision().is_some()`), then one spawned mover is placed (its
  position reads back at the authored point). *Control:* the same boot
  with no `--area` file → exit 1 `NavBake`/missing-input, nothing serves.
  *Fails if:* a route is served with no mesh bound.
- **S6 `rejected_messages_log_one_line_each`.** With `--log-file`, send
  one oversized frame, one malformed frame and one unauthenticated intent
  (three of T042a's eight decoder kinds): the log file holds exactly three
  `rejected <ordinal> <code> <detail>` lines in ordinal order, stderr
  holds the same three lines, and the server keeps serving (exit is not
  taken). *Control:* send the same three frames with no `--log-file` →
  stderr holds the three lines and no file is created. *Fails if:* any
  refusal panics, prints a credential, or logs zero or two lines.
- **S7 `shutdown_is_clean`.** Start the deterministic server, stop it:
  exit 0; with `--save-on-shutdown`, the save path holds a fresh
  `HOSTCKPT` envelope (`tasks/T039.md:115-117`) whose header is
  `01 00 00 00` followed by the campaign id (the T039 vector shape,
  `tasks/T039.md:582-592`). *Control:* kill the save directory's write
  permission → exit 1 with the `Persist` text. *Fails if:* staged work is
  committed on the way out.
- **C1 `gate_12_jobs_green`.** T040b: `dedicated-windows` and
  `dedicated-linux` both green on the PR legs; both artifacts download and
  `crpg-server --help` exits 0 on its native runner; the BR-21 query still
  passes (no godot package in the default graph,
  `.github/workflows/ci.yml:150-174`). *Control:* the same tree without
  T040b has no dedicated-server job (fact sheet §CI). *Fails if:* either
  leg is skipped while `check` runs (no tolerance or missing-baseline
  skips, `AGENTS.md:20-24`).

## What must not change

- Goldens, byte for byte: every `crates/crpg-sim/tests/goldens/*` pair,
  every `crates/crpg-testkit/goldens/*` file (the T153 list,
  `tasks/T153.md:634-637`), unchanged by a binary-only task.
- Every existing `crpg-server` suite passes unchanged: `host_capture`,
  `host_save`, `host_quic` and the `run_*` suites; no `Host`, `QuicHost`,
  `save`, `run` or `runner` signature, cap, error variant or `Display`
  text changes.
- `crates/crpg-server/Cargo.toml`, `Cargo.lock`, `deny.toml`,
  `rust-toolchain.toml`, `crpg-contracts`: untouched by T040a (std only,
  Reading 7); T040b touches only `.github/workflows/ci.yml`, additively.
- The `check` and `client` jobs keep their gates and their BR-21 query
  (`.github/workflows/ci.yml:150-174`); S001 holds (self-hosted runners
  never take `pull_request` jobs, `.github/workflows/ci.yml:24-30`).
- `crates/crpg-server/src/lib.rs`, `src/main.rs:1-2` attributes, and the
  no-`crpg-data`-edge rule (`crates/crpg-server/AGENTS.md:151-154`):
  campaign values stay trusted caller facts.

## Dependency edges

| Edge | Section | `ALLOWED` |
|---|---|---|
| none new (T040a, `std` only) | — | `crpg-server` row is `(None, {"crpg-godot", "crpg-edit"})` (`tools/lint/deps.py:69`); no row change, no lock change, no `deps.py` edit |
| none new (T040b, YAML only) | — | no manifest edge at all |

`main.rs` names only `crpg_server::{host, quic, run, runner, save}`,
`crpg_core::Ulid`, and `std`: each is already a normal dependency or the
standard library (`crates/crpg-server/Cargo.toml:16-38`). A future argument
parser or logger would be a new external crate and needs the user's
approval first (root `AGENTS.md:52`; D67a already declined one,
`tasks/DECISIONS-D67.md:5-12`); the std-only alternative above is the
recommendation. `deps.py` is not edited.

## Out of scope

- Any `Host`/`QuicHost`/`save`/`run`/`runner` behaviour change; the
  incarnation counter source (a persisted monotonic counter, T039 Q9).
- Building hosts from campaign data and gate-10 fixture-campaign breadth
  (carried to T040/T041 by T039 Q6, `tasks/T039.md:829-835`); save
  slots/autosave policy (T039 Q8).
- Gate 13 smokes and the 5,000-tick desync suite (T041,
  `docs/IMPLEMENTATION_PLAN.md:248`); the malicious-client suite itself
  (T042a, done); the hostile-server suite (T042b).
- Reconnect grace expiry tuning and the v4 lane-1 resume follow-ups
  (`tasks/BACKLOG.md:119-121` rows); live migration of saves across OSes
  (T039 Q12: no cross-target claim).
- OS service installation, Windows service control handling, systemd
  units, port-forwarding guides (T090); the public release builds (T089,
  `docs/IMPLEMENTATION_PLAN.md:374`).
- GM/admin protocol (T085); listen-server mixed mode (D65b's later task).

## Open questions (each with a recommendation)

1. **Incarnation source.** T039 leaves the greater-incarnation as a caller
   fact (`tasks/T039.md:845-851`) and the trap repeats it
   (`crates/crpg-server/AGENTS.md:302`). Should T040a persist a monotonic
   counter beside the save directory (recommended: a `u64` LE file bumped
   before each load, so an older save can never reuse epochs), or take
   `--incarnation` only and fail closed when absent?
2. **Config file format.** TOML needs a parser crate; JSON is already in
   the graph (`serde_json` is a `crpg-server` dependency,
   `crates/crpg-server/Cargo.toml:32-33`). Recommend JSON with the exact
   `ServerConfig` field names, so no new edge is needed.
3. **Certificate/key and invitation file formats.** T023b leaves loading,
   generation, format, configuration and tooling to T040
   (`tasks/T023b.md:1369-1371` in the fact sheet). Recommend PEM for
   cert/key (std-readable bytes, no TLS crate in `main.rs`) and a small
   JSON invitation file; exact bytes pinned in the final contract.
4. **Area bounds source.** The area file feeds `bind_area_bounds`
   (`crates/crpg-server/src/host.rs:4611-4621`). Recommend one JSON file
   with `min`/`max` metre triples resolved through the campaign locale
   like `crpgc demo` (its `Show` path), not raw indices; `crates/crpg-server`
   still gains no `crpg-data` edge.
5. **Operator settings default.** `HostSettings::multiplayer()` vs
   `single_player()` bits (`crates/crpg-server/AGENTS.md` via
   `docs/architecture/crpg-server.md:243-251`). Recommend
   `multiplayer()` (dedicated worlds are operator-only,
   `docs/adr/0029-kernel-range-and-time-model.md:295`), with a
   `--single-player-pause` flag mapping to `single_player()`.
6. **Shutdown save.** Should a clean shutdown always write
   `save_host_file`, or only with `--save-on-shutdown`? Recommend
   flag-only (B-8): an always-save hides operator error and doubles
   shutdown latency on large authorities.
7. **Deterministic-mode scope.** Spec names replay inputs, fixed seed and
   a hash per N ticks (`docs/CRPG_ENGINE_SPEC.md:958`). Recommend T040a
   ships exactly that; fixture-campaign completion stays T041's gate-13
   smokes (`docs/CRPG_ENGINE_SPEC.md:1334-1335`).

## Agent log

- 2026-11-19 (UTC) · opencode-go/muse-spark-1.3-contributor + T040 draft b · Opened the blind draft skeleton so the contract sections can be filled one edit at a time.
