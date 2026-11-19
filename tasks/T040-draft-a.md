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

## Behaviour

## Errors

## Tests

## What must not change

## Dependency edges

## Definition of done

## Out of scope

## Open questions

## Manager edits

## Agent log
