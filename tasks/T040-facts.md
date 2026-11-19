# T040 facts — dedicated server binary contract inputs

## Requirements

Binding sentences on the dedicated server, its binary, command line, configuration, logging, service behaviour, and gate 12:

- `crpg-server` binary contents (`docs/CRPG_ENGINE_SPEC.md:150`) — the table row says the binary contains core, rules, sim, script VM, AI, net, persistence; renders nothing; is authoritative.
- `Windows/MSVC supports all four; Linux/GNU supports the headless dedicated server and CLI/tooling` (`docs/CRPG_ENGINE_SPEC.md:144-146`) — the planned product surface is three binaries plus one CLI.
- `Windows single-player embeds the same platform-neutral authoritative server implementation in-process` (`docs/CRPG_ENGINE_SPEC.md:155-156`) — embedding rule for single player.
- `Windows and Linux dedicated processes wrap that same implementation` (`docs/CRPG_ENGINE_SPEC.md:157`) — dedicated processes reuse, not reimplement.
- `Linux headless never imports Godot` (`docs/CRPG_ENGINE_SPEC.md:168`) — Godot-free Linux headless.
- `one platform-neutral authoritative implementation, with Windows in-process single-player hosting and Windows/Linux dedicated process adapters` (`docs/CRPG_ENGINE_SPEC.md:915-917`) — server architecture shape.
- `OS-specific adapters stay above core/rules/sim, and all headless server surfaces remain Godot-free` (`docs/CRPG_ENGINE_SPEC.md:925-926`) — where OS-specific code may live.
- `--headless-deterministic` mode (`docs/CRPG_ENGINE_SPEC.md:958`) — no network, replay-file inputs, fixed seed, a state hash per N ticks.
- `12. build Windows client + editor + server and Linux headless server artifacts` (`docs/CRPG_ENGINE_SPEC.md:1333`) — gate 12 verbatim.
- `smoke tests: Windows embedded + dedicated server, Linux dedicated server, each with a scripted client completing the fixture campaign` (`docs/CRPG_ENGINE_SPEC.md:1334-1335`) — gate 13 verbatim.
- `Windows client/editor/server and Linux headless server artifacts plus Windows embedded-server and dedicated-server smokes and a Linux dedicated-server smoke` (`docs/CRPG_ENGINE_SPEC.md:1345-1347`) — future product acceptance requirements.
- `Linux headless gates must not depend on Godot` (`docs/CRPG_ENGINE_SPEC.md:1348`) — headless gate constraint.
- `Windows embedded and dedicated adapters and the Linux dedicated adapter, including scripted-client fixture completion and the unchanged authority boundary` (`docs/CRPG_ENGINE_SPEC.md:1480-1482`) — authoritative hosting test scope.
- `Every one of these must produce a clean rejection and a log line, never a panic` (`docs/CRPG_ENGINE_SPEC.md:1488`) — malicious-client handling requires a log line.
- `one reusable authoritative host serves Windows embedded single-player, Windows dedicated, and Linux dedicated adapters` (`docs/CRPG_ENGINE_SPEC.md:1589-1591`) — Phase 4 platform acceptance.
- `public Windows/MSVC and Linux/GNU dedicated-server builds` (`docs/CRPG_ENGINE_SPEC.md:1624`) — Phase 10 deliverable.
- `Linux dedicated server, headless tooling, and server-side extensibility remain fully supported` (`docs/CRPG_ENGINE_SPEC.md:1634-1635`) — Phase 12 support scope.
- `Windows/MSVC is primary for development, product, release gating, and behavioural baselines` (`docs/adr/0012-windows-primary-platform.md:17-18`) — primary target including dedicated server and CLI.
- `Linux/GNU is fully supported for dedicated server, headless CLI/tooling, server-side extensibility, and CI/testing` (`docs/adr/0012-windows-primary-platform.md:20-21`) — Linux support scope.
- `Linux headless support must not depend on Godot` (`docs/adr/0012-windows-primary-platform.md:23-24`) — Linux headless constraint.
- `One platform-neutral authoritative server implementation is hosted in-process for Windows single-player and by dedicated processes on Windows and Linux` (`docs/adr/0012-windows-primary-platform.md:25-27`) — hosting rule.
- `Platform-specific process, filesystem, service, and presentation concerns stay above crpg-core, crpg-rules, and crpg-sim` (`docs/adr/0012-windows-primary-platform.md:28-29`) — OS-logic placement rule.
- `Future product gates must require real Windows client/editor/server and Linux headless server artifacts` (`docs/adr/0012-windows-primary-platform.md:99-100`) — future gate obligation.
- `One platform-neutral host in crpg-server, in-memory first (Windows embedded and Windows/Linux dedicated share one authority)` (`docs/adr/0021-authoritative-host-capture.md:32-34`) — single-implementation rule.
- `a [lib] target beside the thin dedicated binary` (`docs/adr/0022-host-session-capture-retirement.md:64`) — library-plus-binary packaging.
- `These choices bind the dedicated server (T040) and reconnect (T025b)` (`docs/adr/0026-quic-host-adapter.md:23-25`) — adapter decisions constrain T040.
- `a dedicated persistent world, the operator only` (`docs/adr/0029-kernel-range-and-time-model.md:295`) — pause rights default for dedicated worlds.
- `dedicated worlds are operator-only` (`docs/adr/0029-kernel-range-and-time-model.md:506-508`) — multiplayer default-rights recommendation.
- `The client trusts exactly one server certificate: the one whose DER hashes (SHA-256) to the pin it received out of band` (`docs/adr/0025-lane0-quic-mapping-and-pinned-trust.md:53-55`) — pinned-trust rule.
- `No webpki roots, no platform verifier and no TOFU are used, and there is no hostname, validity-period or revocation check` (`docs/adr/0025-lane0-quic-mapping-and-pinned-trust.md:57-59`) — what pinned trust omits.
- `Windows and Linux dedicated-server artifacts build (gate 12)` (`docs/IMPLEMENTATION_PLAN.md:179`) — M4 exit criterion 6.
- `Smoke tests pass for Windows embedded, Windows dedicated and Linux dedicated (gate 13)` (`docs/IMPLEMENTATION_PLAN.md:180-181`) — M4 exit criterion 7.
- `T040` row (`docs/IMPLEMENTATION_PLAN.md:245`) — Dedicated binary (Windows plus Godot-free Linux); CI artifact jobs for gate 12.
- `T089` row (`docs/IMPLEMENTATION_PLAN.md:374`) — public Windows/MSVC and Linux/GNU dedicated-server release builds.
- Platform invariant (`AGENTS.md:6-12`) — Windows primary; Linux fully supported for dedicated server and headless CLI/tooling; Linux headless must not depend on Godot.
- `One platform-neutral authoritative server implementation serves Windows embedded single-player and Windows/Linux dedicated hosts` (`AGENTS.md:13-16`) — authority boundary rule.
- `Keep OS-specific process, filesystem, service, and presentation logic above crpg-core, crpg-rules, and crpg-sim` (`AGENTS.md:17-19`) — OS-logic placement rule.
- `Leave the program to T040` (`tasks/DECISIONS-D66.md:18`) — the runnable program waits for T040.
- `T154 builds everything a dedicated server needs to run the loop, and proves a single-player save runs on it` (`tasks/DECISIONS-D66.md:20`) — what T040 inherits.
- `Actually writing the line becomes the server program's job (T040 or T041)` (`tasks/DECISIONS-D67.md:9`) — rejected-message log line ownership.
- Server-log-line backlog row (`tasks/BACKLOG.md:341`) — log line for each rejected client message belongs to T040 or T041 with a where-do-logs-go decision.
- `T040: call bind_area_bounds at area load and place after transfers and spawns (T037 Reading 5)` (`tasks/BACKLOG.md:344`) — movement calls T040 must wire.

## The run loop it would host

Public items of `run.rs` (T154a–c, all built), one line each with signature, plus what T154d adds (contract only, not built):

- `TICK_PERIOD_MS` (`crates/crpg-server/src/run.rs:29`) — `pub const TICK_PERIOD_MS: u64 = 50`, the 20 Hz cycle period.
- `MAX_RETIRED_RECORDS` (`crates/crpg-server/src/run.rs:33`) — `pub const MAX_RETIRED_RECORDS: usize = 64`, cap on retired-link records.
- `Hosting` (`crates/crpg-server/src/run.rs:36`) — `pub trait Hosting` with `type Error` and the five stages `intake`, `drive_ai`, `advance`, `deliver`, `retire_captures`.
- `CycleReport` (`crates/crpg-server/src/run.rs:60`) — `pub struct CycleReport { pub now_ms: u64, pub advance: AdvanceReport }`.
- `StepReport` (`crates/crpg-server/src/run.rs:69`) — `pub struct StepReport { pub cycle: Option<CycleReport>, pub missed: u64 }`.
- `LoopError` (`crates/crpg-server/src/run.rs:78`) — `pub enum LoopError<E> { TimeRegression { last_ms, now_ms }, Hosting(E) }`.
- `HostLoop` (`crates/crpg-server/src/run.rs:110`) — `pub struct HostLoop<H: Hosting>` over one hosting on a slot grid.
- `HostLoop::new` (`crates/crpg-server/src/run.rs:127`) — `pub fn new(hosting: H, start_ms: u64) -> Self`, 50 ms period, first slot at `start_ms`.
- `HostLoop::with_period` (`crates/crpg-server/src/run.rs:139`) — same with an explicit `NonZeroU64` period.
- `HostLoop::step` (`crates/crpg-server/src/run.rs:154`) — runs at most one cycle at `now_ms`, skipping missed slots.
- Loop accessors (`crates/crpg-server/src/run.rs:195-232`) — `period_ms`, `next_due_ms`, `cycles`, `missed`, `last_ms`, `hosting`, `hosting_mut`, `into_hosting`.
- `AiHook` (`crates/crpg-server/src/run.rs:237`) — `pub type AiHook = Box<dyn FnMut(&mut Host, u64) -> Result<(), HostError> + Send>`.
- `LinkStats` (`crates/crpg-server/src/run.rs:244`) — `#[non_exhaustive]` per-link counters (`received`, `refused`, `held`, `sent`).
- `LinkStats::held_moves` (`crates/crpg-server/src/run.rs:254`) — T154c field counting lane-1 frames held for retry.
- `LocalError` (`crates/crpg-server/src/run.rs:259`) — `pub enum LocalError { UnknownPeer, AlreadyAttached }`.
- `LinkError` (`crates/crpg-server/src/run.rs:279`) — `pub enum LinkError { Full, Closed }`.
- `LocalHost` (`crates/crpg-server/src/run.rs:332`) — the in-memory hosting: one `Host`, link table, retired list, AI hook.
- `LocalHost::new` (`crates/crpg-server/src/run.rs:347`) — hosting over `host` with no links and no hook.
- `LocalHost::attach` (`crates/crpg-server/src/run.rs:361`) — attaches a peer with two bounded queues, returning `LocalClient`.
- `LocalHost::set_ai` (`crates/crpg-server/src/run.rs:396`) — sets or clears the AI hook.
- `LocalHost::link_stats` (`crates/crpg-server/src/run.rs:401`) — per-link counters or `None`.
- `LocalHost::take_retired` (`crates/crpg-server/src/run.rs:408`) — drains retired-link records in order, at most the cap.
- `LocalHost::retired_dropped` (`crates/crpg-server/src/run.rs:413`) — total records dropped off a full retired list.
- `LocalHost::host/host_mut/into_host` (`crates/crpg-server/src/run.rs:418-428`) — trusted reads, trusted operator calls, taking the host back out.
- `LocalClient` (`crates/crpg-server/src/run.rs:818`) — the client end of a link: `Send`, not `Clone`, not `Sync`.
- `LocalClient::is_closed` (`crates/crpg-server/src/run.rs:830`) — reads the server-end-gone flag.
- `Transport for LocalClient` (`crates/crpg-server/src/run.rs:834`) — `send`/`recv` over the bounded queues with `LinkError`.
- `Clock` (`crates/crpg-server/src/runner.rs:20`) — `pub trait Clock: Send + 'static { fn now_ms(&self) -> u64; }`.
- `SystemClock` (`crates/crpg-server/src/runner.rs:28`) — wall-clock: origin plus whole milliseconds since construction.
- `SystemClock::new/starting_at` (`crates/crpg-server/src/runner.rs:38-44`) — constructors; pass the loop's `start_ms` as origin.
- `Clock for SystemClock` (`crates/crpg-server/src/runner.rs:59`) — `now_ms` saturates instead of wrapping.
- `spawn` (`crates/crpg-server/src/runner.rs:88`) — drives a `HostLoop` on a thread named `crpg-server-loop`.
- `RunnerHandle` (`crates/crpg-server/src/runner.rs:136`) — loop-thread owner: rendezvous with `call`, stop with `stop`/`join`.
- `RunnerHandle::submit/call/stop/is_finished/join` (`crates/crpg-server/src/runner.rs:146-188`) — run a closure after the next step; wait for it; end the thread; poll/consume the exit.
- `RunnerExit` (`crates/crpg-server/src/runner.rs:211`) — the loop as its last step left it plus the ending step error.
- `RunnerGone` (`crates/crpg-server/src/runner.rs:221`) — calling a runner that already ended.
- `RunnerPanicked` (`crates/crpg-server/src/runner.rs:235`) — joining after the loop thread died.
- T154d `QuicAiHook` (`tasks/T154.md:407`) — `pub type QuicAiHook = Box<dyn FnMut(&mut QuicHost, u64) -> Result<(), QuicHostError> + Send>`.
- T154d `QuicHosting` (`tasks/T154.md:409-416`) — dedicated hosting over one `QuicHost` with `new`, `set_ai`, `quic`, `quic_mut`, `into_quic`, and a `Hosting` impl.
- T154d `QuicHost::flush` (`tasks/T154.md:421`) — runs `pump` phases 4–7 alone so deliver is a stage.
- T154d pass-throughs (`tasks/T154.md:423-428`) — `drive_ai`, `pause`, `unpause` delegating to the host.
- T154d save-into-dedicated proof (`tasks/T154.md:220-236`) — a single-player save loads into the dedicated hosting unchanged and runs, stepped by hand and on the runner.
- T154 handoff to T040 (`tasks/T154.md:245-252`) — `main.rs` stays T040's: certificate, invitations, save identity and incarnation are its wiring.

## Host start-up today

How a `Host` is built, bound, and started, what it needs, and the checkpoint/save adapter (T039):

- `Host::new` (`crates/crpg-server/src/host.rs:973`) — empty authority with no encounter; gameplay needs the `from_history` path.
- `Host::from_history` (`crates/crpg-server/src/host.rs:980`) — wraps an embedding-choreographed authority; refuses a non-empty sim journal.
- `Host::bind_area_bounds` (`crates/crpg-server/src/host.rs:4611`) — bakes an area's two bounds into a mesh and binds it; T040 calls this at area load.
- `Host::nav_revision` (`crates/crpg-server/src/host.rs:4625`) — bound mesh revision, `None` before binding and after a restore.
- `Host::place` (`crates/crpg-server/src/host.rs:4636`) — trusted placement of one entity; T040 calls this after transfers and spawns.
- `bind_nav_mesh` places off-mesh movers; later-created movers are the caller's to `place` (`tasks/T037.md:160-165`) — T037 Reading 5 behind the backlog row.
- `start_encounter` and encounter spawning are T040's (`tasks/T037-draft-a.md:464`) — movement-implied startup work filed at T040.
- `QuicHost::start` (`crates/crpg-server/src/quic.rs:566`) — takes a host plus a server from `bind`/`bind_dual`, returning both on failure.
- `QuicHost::start_resumable` (`crates/crpg-server/src/quic.rs:578`) — same for a server from any `bind*`, `bind_resumable` included.
- `QuicHost::is_resumable` (`crates/crpg-server/src/quic.rs:587`) — reports which route built the adapter.
- Dedicated server binds `bind_resumable` and calls `start_resumable` (`tasks/T025b.md:862-863`) — manager edit T040's row implies.
- Invitation credential is opaque, 16..=128 bytes, length-checked by the transport, judged by the host adapter (`docs/adr/0025-lane0-quic-mapping-and-pinned-trust.md:65-69`) — what QUIC auth needs.
- Certificate chain refused; rotation means a new invitation (`docs/adr/0025-lane0-quic-mapping-and-pinned-trust.md:56-59`) — certificate facts for T040's loading.
- Certificate/key loading, credential generation, invitation format, operator configuration and tooling are T040's (`tasks/T023b.md:1369-1371`) — explicitly out of the adapter's scope.
- T040 needs a certificate, an invitation file, a save to load, and settings (`tasks/DECISIONS-D66.md:20`) — the four inputs named by D66b.
- Campaign path: the crate has no `crpg-data` edge, so identity values are trusted caller facts (`crates/crpg-server/AGENTS.md:151-154`) — no campaign loading exists today.
- `save` module interface (`tasks/T039.md:138`) — §2 exact public Rust for the save adapter.
- `encode_host_save` (`tasks/T039.md:203`) — builds the §3 header plus verbatim checkpoint without inspecting it.
- `decode_host_save` (`tasks/T039.md:207-212`) — validates and loads a checkpoint with a new incarnation.
- `save_host_file` (`tasks/T039.md:215`) — saves a quiescent host through `crpg_persist::save_file`.
- `load_host_file` (`tasks/T039.md:218-223`) — loads a host through `crpg_persist::load_file`.
- Save payload layout (`tasks/T039.md:286`) — §3 normative binary header format.
- Save behaviour (`tasks/T039.md:310`) — §4 exact encode/decode steps and precedence.
- `encode_host_save` as built (`crates/crpg-server/src/save.rs:165`) — header then checkpoint verbatim, `TooLarge` before touching the filesystem.
- A loaded host is unbound and `Restore`-paused under a greater incarnation (T154 Reading 11, save-into-dedicated half) (`tasks/T154.md:224-228`) — what start-up must unpause and rebind.

## Existing binaries and their layout

Every `[[bin]]`, every `src/bin/*.rs`, the `crpgc` structure, E012, and binary test discipline:

- `crpg-server` `[[bin]]` (`crates/crpg-server/Cargo.toml:12-14`) — binary name `crpg-server`, entry `src/main.rs`.
- `crpg-server` `[lib]` (`crates/crpg-server/Cargo.toml:9-10`) — library entry `src/lib.rs` beside the thin binary.
- No `src/bin/*.rs` anywhere in the workspace (glob `crates/*/src/bin/*.rs` finds none) — both binaries are package-root `src/main.rs` files.
- `crpgc` `[[bin]]` (`crates/crpg-cli/Cargo.toml:26-28`) — binary name `crpgc`, entry `src/main.rs`.
- `crpgc` sources (`crates/crpg-cli/src`) — `main.rs` plus `apply.rs`, `combat_apply.rs`, `demo.rs`, `play.rs` command modules.
- `crpg-server` binary refuses to run (`crates/crpg-server/src/main.rs:9-12`) — prints "dedicated adapter not yet implemented" and exits 1.
- The binary is a wiring shell with no host policy (`crates/crpg-server/AGENTS.md:24-26`) — policy lives in the library.
- Seven library modules, no glob re-export (`crates/crpg-server/src/lib.rs:8`) — `host`, `capture`, `checkpoint`, `save`, `quic`, `run`, `runner`.
- Client/editor are Godot projects over `crpg-godot`, never Rust packages (`tasks/E012-binary-crate-naming.md:65-68`) — resolved binary naming.
- `crpg-server` owns the reusable host as a library target plus a thin dedicated binary (`tasks/E012-binary-crate-naming.md:69-70`) — packaging resolution.
- Spec §14 lists only `crpg-server [bin]` plus `crpg-cli [bin]` plus `crpg-godot [cdylib]` (`tasks/E012-binary-crate-naming.md:7-12`) — the deferred-task naming gap E012 closed.
- Binary tests drive `env!("CARGO_BIN_EXE_crpgc")` as a real process (`crates/crpg-cli/tests/demo.rs:3`) — black-box binary discipline.
- Exit-code plus stdout-byte assertions, never internals (`crates/crpg-cli/tests/demo.rs:3-9`) — what the black-box tests pin.

## CI today

## Rules that bind it

## Agent log
