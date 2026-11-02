# T045 facts — Godot bridge fact sheet

Read-only fact gathering for the T045 bridge ADR and the T047 contract.
Branch `task/T045-facts-f1`, 2026-11-01. No design, no recommendations: every
entry cites a file and line on this branch and states what it says or does.
"none found" means the named thing was searched for and does not exist here.

## Requirements

### Spec `docs/CRPG_ENGINE_SPEC.md`

- `§11.3 heading` (`docs/CRPG_ENGINE_SPEC.md:1008`) — the section is titled "The FFI surface (keep it small)".
- `§11.3 five objects` (`docs/CRPG_ENGINE_SPEC.md:1015`) — "Between Rust and GDScript there are roughly five object types:".
- `§11.3 object list` (`docs/CRPG_ENGINE_SPEC.md:1017`) — `Document` (open/save/apply/undo/validate/query), `Session` (connect/play/intent/query replica), `Assets` (import/scan/thumbnail), `Diagnostics` (list/subscribe), `Preview` (bake nav, compute LOS, ranges).
- `§11.3 everything else` (`docs/CRPG_ENGINE_SPEC.md:1019`) — "Everything else stays in Rust."; a small FFI surface is what makes the engine-swap escape hatch real.
- `§11.3 provenance note` (`docs/CRPG_ENGINE_SPEC.md:1010`) — E022 ledgers owners/prerequisites/phases of the FFI, replica-query, edit-command, session and distribution shapes in `tasks/BACKLOG.md` ("API-shape ledger"); the verbs are design intent "until their owning task pins exact signatures" (`:1013`).
- `§19.1 item 22` (`docs/CRPG_ENGINE_SPEC.md:1666`) — "A Godot scene that spawns N `Node3D` proxies from an external array."
- `§19.1 item 23` (`docs/CRPG_ENGINE_SPEC.md:1667`) — "The client's interpolation buffer with a unit test."
- `§19.2 item 12` (`docs/CRPG_ENGINE_SPEC.md:1686`) — "Scene sync on the client with proxy lifecycle."
- `GDExtension consumption` (`docs/CRPG_ENGINE_SPEC.md:39`) — "Godot ships as a pinned upstream tag plus a small patch queue, consumed through GDExtension. Not a fork."
- `client project path` (`docs/CRPG_ENGINE_SPEC.md:855`) — "crpg-client (Godot 4 application, project apps/client)".
- `layer diagram: bridge row` (`docs/CRPG_ENGINE_SPEC.md:190`) — `crpg-godot [cdylib]` holds "engine-facing replica queries, input encoding, editor FFI, embedded-host adapter".
- `layer diagram: FFI arrow` (`docs/CRPG_ENGINE_SPEC.md:187`) — Presentation (Godot projects) reaches the stack through "GDExtension (godot-rust), narrow FFI surface".
- `nothing Godot below the bridge` (`docs/CRPG_ENGINE_SPEC.md:207`) — "pure Rust, no Godot types below crpg-godot".
- `bridge on top` (`docs/CRPG_ENGINE_SPEC.md:218`) — "`crpg-godot` sits on top because its embedded-host adapter imports the server library".
- `hosting placement` (`docs/CRPG_ENGINE_SPEC.md:163`) — resolved 2026-09-30 (E012, POST-T018 D02, T032): `crpg-server` owns the reusable host as a library; Windows embedded single-player uses it "through a `crpg-godot` adapter" (`:166`); client and editor are Godot projects over `crpg-godot`, "not new Rust packages" (`:167`).
- `replica query split` (`docs/CRPG_ENGINE_SPEC.md:169`) — `crpg-godot` owns the engine-facing query/presentation objects; neither it nor the client gets mutable authoritative access (`:170`).
- `in-memory authority boundary` (`docs/CRPG_ENGINE_SPEC.md:177`) — "The authority boundary is unchanged when transport is in-memory."
- `net_id in server deltas` (`docs/CRPG_ENGINE_SPEC.md:776`) — sketch `EntityEnter { net_id, archetype, initial: FilteredComponents }`, `EntityLeave { net_id }`, `ComponentUpdate { net_id, component_id, payload }` (`:777`, `:778`).
- `net_id in intents` (`docs/CRPG_ENGINE_SPEC.md:794`) — sketch `Interact { net_id }` on the client→server intent list.
- `sketches are not the live wire` (`docs/CRPG_ENGINE_SPEC.md:801`) — as built 2026-09-30, the enums above are design sketches; live shapes are `crates/crpg-net/src/protocol.rs` (v1) and `protocol_v2.rs`.
- `unsafe exception` (`docs/CRPG_ENGINE_SPEC.md:1103`) — "only `crpg-godot` may use `unsafe`"; no loader, dependency or unsafe exception is authorised by T009c.
- `crate list: bridge` (`docs/CRPG_ENGINE_SPEC.md:1204`) — "`crpg-godot/ [cdylib] GDExtension bridge for client + editor, embedded-host adapter`".
- `crate list: client project` (`docs/CRPG_ENGINE_SPEC.md:1206`) — "`client/ Godot project: scenes, UI, GDScript views`" under `apps/`.
- `client rule: no authority` (`docs/CRPG_ENGINE_SPEC.md:879`) — "No authoritative logic."; the client may compute predictions and display hints but must recompute nothing that determines an outcome.
- `client rule: query API` (`docs/CRPG_ENGINE_SPEC.md:881`) — "UI reads the replica world through a stable query API", not through Godot nodes; nodes exist only for rendering and picking.
- `§9.1 Rust-side contents` (`docs/CRPG_ENGINE_SPEC.md:856`) — the GDExtension side holds the net client, `ReplicaWorld` (read-only replica mode, `:858`), interpolation buffer and own-movement prediction, intent encoding plus rate limiting, and the query API exposed to GDScript (`:861`).
- `phase 0 risk` (`docs/CRPG_ENGINE_SPEC.md:1542`) — "**Risks:** GDExtension performance for many skinned meshes; `godot-rust` API friction."
- `phase 5 deliverables` (`docs/CRPG_ENGINE_SPEC.md:1590`) — "`crpg-godot` bridge, scene sync, camera, click-to-move with prediction, basic HUD, combat log, dialogue UI, `SimEvent`-driven VFX and audio hooks."
- `gate 12` (`docs/CRPG_ENGINE_SPEC.md:1325`) — "build Windows client + editor + server and Linux headless server artifacts".
- `gates are capability-gated` (`docs/CRPG_ENGINE_SPEC.md:1332`) — steps 7–13 are capability-gated, not placeholder jobs; product acceptance needs real artifacts and "Linux headless gates must not depend on Godot" (`:1340`).
- `technical decision 1` (`docs/CRPG_ENGINE_SPEC.md:1721`) — pinned Godot 4.x + patch queue, consumed via GDExtension; core is Godot-free; risk "GDExtension API gaps".
- `technical decision 2 risk` (`docs/CRPG_ENGINE_SPEC.md:1722`) — Rust core risk list includes "gdext maturity".
- `risk 17` (`docs/CRPG_ENGINE_SPEC.md:1772`) — `godot-rust` (gdext) abandonment/breaking changes; mitigation "Small FFI surface. Pin the version." with a C++ GDExtension bridge as fallback.
- `CI lint obligation` (`docs/CRPG_ENGINE_SPEC.md:1887`) — put the rule in `AGENTS.md` and "enforce it with a CI lint on `crpg-godot` being the only crate allowed to depend on `godot`".

### `tasks/E012-binary-crate-naming.md` Resolution

- `E012 resolution` (`tasks/E012-binary-crate-naming.md:65`) — "**Naming and placement resolved (option a).** Client and editor are Godot projects (`apps/client`, `apps/editor`) over the `crpg-godot` GDExtension crate ... they are never Rust packages."
- `no bridge crate` (`tasks/E012-binary-crate-naming.md:68`) — "There is no `crpg-client-bridge` crate: the bridge is `crpg-godot`."
- `host library plus adapter` (`tasks/E012-binary-crate-naming.md:69`) — `crpg-server` owns the reusable host as a library target plus a thin dedicated binary; embedded single-player uses it "through a `crpg-godot` adapter"; "Linux headless never imports Godot" (`:71`).
- `ReplicaWorld split` (`tasks/E012-binary-crate-naming.md:72`) — `crpg-net` owns filtered replica storage, delta application and engine-neutral queries; `crpg-godot` owns engine-facing query/presentation objects.
- `no ALLOWED change` (`tasks/E012-binary-crate-naming.md:74`) — "No new package and no `ALLOWED` change."
- `still open` (`tasks/E012-binary-crate-naming.md:81`) — E022 owns the exact host/editor/FFI interfaces; E023 native extensions stay deferred; "Nothing here is implemented yet."

### `docs/IMPLEMENTATION_PLAN.md` rows

- `T045` (`docs/IMPLEMENTATION_PLAN.md:247`) — "Bridge ADR: §11.3 FFI surface, threading, embedded host via `crpg-server`, error mapping, `net_id`↔`EntityId` view, `client/` project layout (E012/E022)".
- `T047` (`docs/IMPLEMENTATION_PLAN.md:249`) — "Bridge skeleton: gdext extension, `client/` project, embedded host, proxy spawn from a bulk snapshot (§19.1 #22), Windows client build (gate 12). Opens the crate. H-unsafe"; prerequisites T045, T046, D27d, T105; kind C → I, S.
- `T048` (`docs/IMPLEMENTATION_PLAN.md:250`) — "Scene sync and proxy lifecycle (§19.2 #12)", prerequisite T047.
- `T049` (`docs/IMPLEMENTATION_PLAN.md:251`) — "Interpolation buffer (§19.1 #23)", prerequisite T047.
- `T051` (`docs/IMPLEMENTATION_PLAN.md:254`) — "HUD, combat log, `SimEvent`-driven VFX/audio hooks", prerequisites T048, T104, T108.
- `T104` (`docs/IMPLEMENTATION_PLAN.md:262`) — player UX research then a clickable combat-HUD prototype; "findings become T050/T051 acceptance criteria", needed before the T050/T051 contracts.
- `T105` (`docs/IMPLEMENTATION_PLAN.md:263`) — asset sourcing per spec risk 15 (licence-compatible glTF packs, one humanoid rig standard); human buy gate; "before T047".
- `§1.4 S-set line` (`docs/IMPLEMENTATION_PLAN.md:95`) — "the GDExtension boundary in `crpg-godot`" is in the serialized, human-reviewed set; packages marked S in §3 are in that set (`:98`).
- `M4 exit gate 12` (`docs/IMPLEMENTATION_PLAN.md:179`) — "Windows and Linux dedicated-server artifacts build (gate 12)."
- `T040 gate-12 job` (`docs/IMPLEMENTATION_PLAN.md:235`) — dedicated binary "(Windows plus Godot-free Linux); CI artifact jobs for gate 12".
- `M6 exit` (`docs/IMPLEMENTATION_PLAN.md:317`) — "all twelve spec §17.2 items demonstrated, with gates 10, 12 and 13".

### `tasks/DECISIONS-2026-10-04.md` D27d

- `D27d binding` (`tasks/DECISIONS-2026-10-04.md:69`) — "the `godot` crate, current stable when T047's contract is written (0.5.5 on 2026-10-04), exact pin, `crpg-godot` only."
- `D27d engine` (`tasks/DECISIONS-2026-10-04.md:70`) — "the Godot 4 build installed on the maintainer's Windows machine (`Godot4 --version`), recorded in T047's completion record and pinned per ADR-0001."

## Earlier spikes

- `ADR-0001 decision` (`docs/adr/0001-godot-pinned-not-forked.md:13`) — "Use Godot as an unmodified, version-pinned presentation host, consumed through GDExtension. Keep the entire simulation in Rust crates with no Godot dependency."
- `ADR-0001 consequences` (`docs/adr/0001-godot-pinned-not-forked.md:19`) — the server can run with no graphics stack; Godot upgrades stay cheap; replacing Godot is a client rewrite, not a project rewrite.
- `ADR-0001 costs` (`docs/adr/0001-godot-pinned-not-forked.md:22`) — Godot types must be kept out of the core, "enforced by CI lint"; two languages in the client (Rust core, GDScript views).
- `ADR-0003 spike location` (`docs/adr/0003-gdextension-rendering-spike.md:15`) — throwaway spike in `C:\CRPG\Dev\spike-gdext`, a separate repo with no dependency on the workspace.
- `ADR-0003 what was built` (`docs/adr/0003-gdextension-rendering-spike.md:17`) — Rust `CharacterHerd` GDExtension class, an array of up to 1,000 `{position, yaw, anim_state}` structs, advanced at a fixed 20 Hz in `_physics_process`.
- `ADR-0003 bulk snapshot` (`docs/adr/0003-gdextension-rendering-spike.md:20`) — one bulk `snapshot()` call returning current+previous transforms for the whole population as one `PackedFloat32Array`.
- `ADR-0003 Godot side` (`docs/adr/0003-gdextension-rendering-spike.md:22`) — `HerdMain.gd` spawns N `CesiumMan.glb` (CC BY 4.0) instances, interpolates between the last two 20 Hz ticks and drives `AnimationPlayer`; no gameplay logic in GDScript.
- `ADR-0003 instrumentation` (`docs/adr/0003-gdextension-rendering-spike.md:28`) — 2 s warmup, then averages fps and per-frame FFI cost over an 8 s window and prints one result line, no manual counter reading.
- `ADR-0003 measurements` (`docs/adr/0003-gdextension-rendering-spike.md:37`) — 200 chars: 231.7 fps / 87.4 µs per frame; 500: 88.9 fps / 199.8 µs; 1,000: 43.7 fps / 394.1 µs (RTX 4060 Laptop, release build, vsync off).
- `ADR-0003 decision` (`docs/adr/0003-gdextension-rendering-spike.md:42`) — go; the spec gate (200 characters at ≥60 fps, FFI under 1 ms per tick) is met with wide margin.
- `ADR-0003 findings` (`docs/adr/0003-gdextension-rendering-spike.md:48`) — FFI cost scales roughly linearly (~0.4 µs/character); bulk-snapshotting the whole population in one call "is the right pattern and should carry forward into the real client bridge".
- `ADR-0003 fps finding` (`docs/adr/0003-gdextension-rendering-spike.md:52`) — fps falls much faster than FFI cost (232 → 89 → 44), consistent with per-character skinning/animation cost, not the FFI boundary.
- `ADR-0003 simplification` (`docs/adr/0003-gdextension-rendering-spike.md:59`) — the spike used `AnimationPlayer.speed_scale`, not a literal `AnimationTree`; deliberate for a one-clip throwaway asset, not load-bearing.
- `T001 spike task` (`tasks/T001.md:23`) — work happens in `C:\CRPG\Dev\spike-gdext`, which "already has a working `godot`-crate toolchain proof, `HelloProbe`, committed as a baseline first".
- `T001 measurement ask` (`tasks/T001.md:30`) — measure frame time at 200, 500 and 1,000 characters on the RTX 4060 laptop, with per-tick FFI cost separate from render cost.
- `spec T1` (`docs/CRPG_ENGINE_SPEC.md:1897`) — the GDExtension rendering spike task; purpose is to prove Godot can render CRPG-scale content driven entirely by external state (`:1898`); done at 200 characters ≥60 fps and FFI under 1 ms per tick (`:1903`).
- `PROJECT_STATE pointer` (`docs/PROJECT_STATE.md:380`) — "T001 GDExtension rendering spike — go (ADR-0003), 200 chars @ 231.7 fps, FFI cost 87.4 µs/frame, on the RTX 4060 laptop"; spike lives in `C:\CRPG\Dev\spike-gdext`, not this workspace (`:382`).
- `day-one version banner` (`docs/DAY_ONE.md:107`) — "Live pins are Godot 4.7.2 / rustc 1.98.0"; the `4.6.x` strings below it are the day-one bootstrap snapshot.
- `day-one toolchain proof` (`docs/DAY_ONE.md:518`) — today's `HelloProbe` proves the toolchain; T1 proves the architecture; "Do not conflate them."
- `godot crate version in the spike` — none found: ADR-0003, T001, `docs/DAY_ONE.md` and `docs/PROJECT_STATE.md` record no `godot` crate version; the only dated version statement is D27d's 0.5.5 (`tasks/DECISIONS-2026-10-04.md:69`).
- `ADR-0012 one host` (`docs/adr/0012-windows-primary-platform.md:25`) — "One platform-neutral authoritative server implementation" is hosted in-process for Windows single-player and by dedicated processes on Windows and Linux.
- `ADR-0012 in-process boundary` (`docs/adr/0012-windows-primary-platform.md:30`) — "The client/server authority boundary is unchanged in-process. Transport may be in-memory, but clients still cannot mutate authoritative state directly."
- `ADR-0012 no Linux GUI promise` (`docs/adr/0012-windows-primary-platform.md:23`) — Linux GUI client/editor builds are not promised, and Linux headless support must not depend on Godot.
- `ADR-0012 exact-build replay` (`docs/adr/0012-windows-primary-platform.md:33`) — replay determinism is exact-build; each target compares its own golden; cross-platform hash equality is not required.
- `ADR-0012 targets` (`docs/adr/0012-windows-primary-platform.md:40`) — current gate targets are `x86_64-pc-windows-msvc` and `x86_64-unknown-linux-gnu`.
- `ADR-0012 E023` (`docs/adr/0012-windows-primary-platform.md:93`) — E023 tracks T0 native loading/packaging and must reconcile target artifacts with "only `crpg-godot` may use `unsafe`"; no loader, dependency, unsafe exception or stable ABI is authorised.

## crpg-godot today

- `crate manifest` (`crates/crpg-godot/Cargo.toml:1`) — package `crpg-godot` version `0.1.0`, workspace edition and licence; the `[dependencies]` table (`:7`) is empty. No `[lib]`, features, build script or dev-dependencies.
- `crate root` (`crates/crpg-godot/src/lib.rs:1`) — `#![allow(unsafe_code)]` (not `forbid`); `#![warn(missing_docs)]` (`:2`); the doc comment says it is "The only crate permitted to use unsafe or to depend on the godot crate" (`:3`).
- `crate contents` (`crates/crpg-godot/src/lib.rs:4`) — the file ends there; there is no other item in `src/` and no `AGENTS.md` or architecture doc in the crate.
- `architecture doc status` (`docs/architecture/README.md:58`) — `crpg-godot`: "due with its first task", referencing spec §9, ADR-0001, ADR-0003.
- `ALLOWED entry` (`tools/lint/deps.py:71`) — `"crpg-godot": (None, set())`: every workspace crate is an allowed dependency and none is excluded.
- `GODOT_CRATE constant` (`tools/lint/deps.py:75`) — `GODOT_CRATE = "crpg-godot"`, described as "The one crate allowed unsafe and the godot dependency".
- `godot exception rule` (`tools/lint/deps.py:12`) — "**godot.** Only `crpg-godot` may depend on it, in any section."
- `unsafe exception rule` (`tools/lint/deps.py:16`) — "**unsafe.** Only `crpg-godot` may use it."
- `godot check` (`tools/lint/deps.py:232`) — `check_godot` fails any non-`crpg-godot` direct dependency on `godot`, including renamed (`package = "godot"`) and target-specific entries; violation text `foo -> godot (godot-only, ...)` at `:245`.
- `unsafe check` (`tools/lint/deps.py:279`) — `check_unsafe` skips `GODOT_CRATE`; every other crate root must contain `#![forbid(unsafe_code)]` (`:88`, `:290`).
- `determinism scope` (`tools/lint/determinism.py:57`) — `CRATES = ["crpg-core", "crpg-rules", "crpg-sim"]`; `crpg-godot` is not scanned. The file contains no `godot` handling.
- `CI matrix` (`.github/workflows/ci.yml:105`) — `check` runs on `ubuntu-latest` and `windows-latest`; workspace gates are `cargo fmt`, `cargo clippy --workspace --all-targets --locked -- -D warnings` and `cargo test --workspace --locked` (`:150`, `:157`, `:159`).
- `CI has no godot job` (`.github/workflows/ci.yml:182`) — the jobs are `guard`, `secrets`, `check`, `deny`, `lint-architecture`, `lint-determinism` and `lint-selftest`; none names Godot, builds a client, or sets `GODOT4_BIN`.
- `CI docs-only skip` (`.github/workflows/ci.yml:138`) — a PR whose changed paths all match `^(docs|tasks)/`, root Markdown or `crates/*/AGENTS.md` skips the Rust gates; pushes and the schedule run everything.
- `Cargo.lock entry` (`Cargo.lock:177`) — `crpg-godot` version `0.1.0` with no `dependencies` list.
- `godot in Cargo.lock` — none found: `Cargo.lock` has 140 packages and no package named `godot` or `godot-*`.
- `vendored godot` — none found: no `vendor/` directory is tracked, no `.gdextension` file exists, and no dependency entry in any `Cargo.toml` or in `deny.toml` names `godot`.
- `client/ project` — none found: there is no `apps/` directory and no tracked file under `apps/`.

## Embedded host surface

- `library modules` (`crates/crpg-server/src/lib.rs:35`) — five public modules, no glob re-export: `capture`, `checkpoint`, `host`, `quic`, `save`.
- `client capability` (`crates/crpg-server/src/lib.rs:29`) — "No client receives `&Host`, `&mut World`, or capture access: the client-facing capability is a copyable `PeerHandle` plus the adapter's byte channel."
- `Host` (`crates/crpg-server/src/host.rs:524`) — `pub struct Host`, one serial owner of `HistoryWorld`; no thread, socket or clock field.
- `construction: empty` (`crates/crpg-server/src/host.rs:549`) — `Host::new(seed: u64, config: HostConfig, now_ms: u64) -> Result<Self, HostError>`; "An empty authority with no encounter; gameplay needs the `from_history` path."
- `construction: wrapped authority` (`crates/crpg-server/src/host.rs:556`) — `Host::from_history(history: HistoryWorld, config: HostConfig, now_ms: u64) -> Result<Self, HostError>`; refuses a non-empty sim journal with `HostError::PendingHistory` (`:562`).
- `host config` (`crates/crpg-server/src/host.rs:112`) — `HostConfig { protocol: ProtocolSelection, incarnation: u64 }`; `ProtocolSelection::V1`/`V2` at `:84`; `as_u8` at `:93`.
- `bind a peer` (`crates/crpg-server/src/host.rs:691`) — `bind_peer(control: ControlGrant, grants: DisclosureGrants, now_ms: u64) -> Result<PeerHandle, HostError>`; stages the first permitted state from delivery seq 1.
- `bind a snapshot peer` (`crates/crpg-server/src/host.rs:704`) — `bind_snapshot_peer` has the same shape but opts the session into the `0x81` transfer subprotocol.
- `change grants` (`crates/crpg-server/src/host.rs:819`) — `set_grants` returns `GrantUpdate` (`Unchanged` / `RebindRequired` / `SnapshotStaged`, `:206`); `set_control` at `:883`.
- `submit intent: ingest` (`crates/crpg-server/src/host.rs:904`) — `ingest(peer: PeerHandle, bytes: &[u8], now_ms: u64) -> Result<IngestDisposition, HostError>`; byte-level admission only (open, time, binding, rates, frame cap, queue room), "Never decodes and never consumes lane sequence."
- `pump` (`crates/crpg-server/src/host.rs:954`) — `pump(now_ms: u64) -> Result<PumpSummary, HostError>`; drains staged commands FIFO through the §3 pipeline and stops at the first failed reservation.
- `pump report` (`crates/crpg-server/src/host.rs:292`) — `PumpSummary` with `admitted == applied + rejected`, `backpressured` 0 or 1 and `dropped_replies`; per-command `AdmissionResult { peer, seq, status, cached, retained_in_ingress }` at `:272`.
- `receive frames` (`crates/crpg-server/src/host.rs:1682`) — `take_delivery(peer) -> Result<Vec<Vec<u8>>, HostError>`; every unacknowledged frame in delivery order, repeated until acknowledged.
- `receive frames with kind` (`crates/crpg-server/src/host.rs:1695`) — `take_delivery_entries(peer) -> Result<Vec<DeliveryEntry>, HostError>`; `DeliveryEntry { ordinal, kind, bytes }` at `:194`.
- `acknowledge delivery` (`crates/crpg-server/src/host.rs:1721`) — `acknowledge_delivery(peer, through: u64)`; `through` is a delivery **ordinal**; past the last is `FutureDeliveryCursor`.
- `trusted tick` (`crates/crpg-server/src/host.rs:1950`) — `tick(&mut self) -> Result<(), HostError>` is one trusted `HistoryWorld::tick` and "journals nothing".
- `shutdown` (`crates/crpg-server/src/host.rs:1969`) — idempotent fence: refuses staged work as `SessionExpired`, closes bindings; authority, captures and save stay available.
- `checkpoint bytes` (`crates/crpg-server/src/host.rs:1961`) — `save_checkpoint(&self) -> Result<Vec<u8>, CheckpointError>`.
- `save files` (`crates/crpg-server/src/save.rs:272`) — `save_host_file(path, host, identity)` and `load_host_file` (`:287`) with `SaveIdentity` (`:89`) and `LoadedSave` (`:98`).
- `no transport type in the host API` (`crates/crpg-server/src/host.rs:904`) — `ingest` takes a raw `&[u8]`; delivery returns raw bytes (`:1682`); the host names neither `Transport` nor any socket type.
- `in-memory transport shape` (`crates/crpg-net/src/transport.rs:14`) — `trait Transport { type Error; fn send(&mut self, bytes: &[u8]); fn recv(&mut self) -> Option<Vec<u8>> }`; bounded in-memory pipe between two endpoints in one process, "No threads, no clock, no I/O" (`:11`).
- `simulated transport` (`crates/crpg-net/src/sim.rs:385`) — `InMemoryTransport`, "Deterministic in-memory delivery fabric for lane-0 bytes" (struct declared at `:396`), with `SimClock` (`:49`) as its only time source.
- `real transport adapter` (`crates/crpg-server/src/quic.rs:466`) — `QuicHost::start` and per-pump `QuicHost::pump(now_ms)` (`:501`) wrap one `Host` plus one `QuicServer`; inbound frames enter the same `ingest` → `pump` path (`:10`).

### The in-process path `crpgc play` and `crpgc demo` actually use

- `no server edge in the CLI` (`crates/crpg-cli/Cargo.toml:7`) — dependencies are `crpg-core`, `crpg-data`, `crpg-sim`, `crpg-testkit`, `serde_json`, `same-file`; there is no `crpg-server` edge and no transport.
- `play construction` (`crates/crpg-cli/src/play.rs:78`) — loads the campaign, builds `Show::from_loaded`, `CombatAdapter::new(loaded)` and `World::new(seed)` in one process.
- `play starts the encounter` (`crates/crpg-cli/src/play.rs:81`) — `adapter.start(&mut world, encounter_id)`.
- `play asks for options` (`crates/crpg-cli/src/play.rs:110`) — options come from `crpg_sim::legal_actions(&world, active)`.
- `play performs one` (`crates/crpg-cli/src/play.rs:128`) — `adapter.perform(&mut world, &chosen)`, then prints through `Show`.
- `adapter start` (`crates/crpg-cli/src/combat_apply.rs:285`) — assembles the transient `EncounterSpec` from loaded documents and calls `crpg_sim::start_encounter(world, &spec)`.
- `adapter perform` (`crates/crpg-cli/src/combat_apply.rs:117`) — calls `crpg_sim::perform_action(world, action)` and maps runtime entities back to authored placements.
- `demo construction` (`crates/crpg-cli/src/demo.rs:62`) — reads the replay through `crpg_testkit::read_replay`, then `play_replay` with an apply closure (`:72`).
- `demo apply closure` (`crates/crpg-cli/src/demo.rs:73`) — `adapter.step(world, payload)` per replay intent, then prints header/attack/turn/summary blocks.
- `nearest Host in-process usage` (`crates/crpg-server/tests/host_capture.rs:508`) — test harness builds `Host::from_history(fixture(), HostConfig { protocol, incarnation }, 0)`, binds (`:529`), advances injected time and `ingest`s (`:549`), `pump`s (`:555`) and `take_delivery`s (`:615`).

## Client data available

- `delta frame fields` (`crates/crpg-net/src/protocol.rs:292`) — `DeltaFrame { lane, server_tick, event_seq, ops }`; `event_seq` is a per-client gapless delivery sequence.
- `v1 delta vocabulary` (`crates/crpg-net/src/protocol.rs:309`) — `EntityEnter`, `EntityLeave`, `Spawned`, `Despawned`, `Died`, `Health { entity, health, max_health, dead }`, `Turn { active, round }` and `Receipt { epoch, lane, seq, processed_tick, status }`.
- `no transform op` (`crates/crpg-net/src/protocol.rs:305`) — "There is no transform op in v1 (spatial replication waits for the lane-1 movement channel) and no open component bag."
- `v2 adds events` (`crates/crpg-net/src/protocol_v2.rs:88`) — `DeltaOp` adds `ActionResolved { actor, target, ability, outcome, damage }` (`:92`), `TurnStarted { actor, round }` (`:107`) and `EncounterEnded { encounter, round }` (`:114`); v1 ops ride as `Legacy` (`:90`).
- `what events omit` (`crates/crpg-net/src/protocol_v2.rs:82`) — the event variants carry "only disclosable identities, symbolic outcomes, reported damage, and widened rounds — never rolls, DCs, margins, grants, entity generations, sim sequences, or internal operation ids."
- `receipt status` (`crates/crpg-net/src/protocol.rs:377`) — `ReceiptStatus::{Applied, Rejected(RejectionCode)}`; duplicates return the original terminal receipt and never re-execute (`:375`).
- `rejection codes` (`crates/crpg-net/src/protocol.rs:393`) — a closed `#[repr(u8)]` enum whose wire numbers are frozen; shape failures use `CodecError` instead.
- `NetId shape` (`crates/crpg-net/src/protocol.rs:217`) — `pub struct NetId(u64)`, documented as a "Client-scoped replica id": zero is never valid, ids are never reused within one epoch, and raw arena ids are never serialized (`:209`).
- `snapshot transfer messages` (`crates/crpg-net/src/protocol_transfer.rs:235`) — downstream `TransferDown::{Begin, Chunk, Cancel}`; upstream `TransferUp::{Published, Resync}` (`:253`).
- `snapshot body` (`crates/crpg-net/src/protocol_transfer.rs:278`) — `SnapshotBody { entities: Vec<SnapshotEntity>, turn: Option<TurnFact> }`.
- `snapshot entity` (`crates/crpg-net/src/protocol_transfer.rs:287`) — `SnapshotEntity { entity: NetId, health: Option<HealthFact> }`; `HealthFact { health, max_health, dead }` (`:296`); `TurnFact { active, round }` (`:307`).
- `host's id maps are private` (`crates/crpg-server/src/host.rs:456`) — `Session { map: BTreeMap<EntityId, NetId>, reverse: BTreeMap<NetId, EntityId>, .. }`; `Session` is private and the crate exposes no accessor (crate contract: "no accessor returns `&HistoryWorld`, `&World` or `&mut` anything", `crates/crpg-server/AGENTS.md:44`).
- `disclosure is deny-by-default` (`crates/crpg-server/AGENTS.md:58`) — unknown entities and flags are denied; a missing field omits the whole event; `net`'s `project_events` does the omission from host-built candidates.
- `snapshot staging` (`docs/architecture/crpg-server.md:202`) — staging builds the body from `PeerView`: "mapped entities, their health, and the turn when granted" — only facts granted at capture.
- `what the wire never carries` (`docs/architecture/crpg-server.md:161`) — per peer the host keeps the `EntityId → NetId` map; receipts go to the originator; permitted events and state ops fan out per peer.
- `campaign load` (`crates/crpg-data/src/loader.rs:62`) — `LoadedCampaign { documents: BTreeMap<SourcePath, Document>, index: BTreeMap<Ulid, IndexEntry> }`; built by `load_campaign` (`:251`).
- `document families` (`crates/crpg-data/src/document.rs:403`) — `Document` is the tagged envelope; it includes `Creature` (`:414`), `Ruleset` (`:432`), `Ability` (`:435`), `Encounter` (`:441`), `Placements` (`:447`) and `Locale` (`:453`).
- `locale table` (`crates/crpg-data/src/document.rs:368`) — `LocaleDocument { campaign, locale, strings: BTreeMap<String, String> }`; document `name` fields are locale keys.
- `ruleset stats` (`crates/crpg-data/src/combat.rs:382`) — `Ruleset { stats: Vec<StatDecl>, health_stat, attributes, pools, abilities, .. }`; `Encounter { ruleset, participants, .. }` at `:634`.
- `display names need the campaign` (`crates/crpg-cli/src/demo.rs:108`) — "Document `name` fields are locale keys, so a key is resolved through the campaign's locale table"; the CLI's `Names` collects campaign, locale, encounters, abilities, placements, creatures and factions (`:112`).
- `client-side stats today` (`crates/crpg-cli/src/play.rs:145`) — play prints each combatant's name plus `health/max_health` from the sim; names come from the loaded campaign, health from the sim's public read.
- `data edge availability` (`tools/lint/deps.py:71`) — `crpg-godot` may import any workspace crate, so `crpg-data`, `crpg-net` and `crpg-server` are all legal edges for the bridge crate.

## Threading and time

- `injected clock in the host` (`crates/crpg-server/src/host.rs:549`) — construction, `bind_peer` (`:691`), `ingest` (`:904`) and `pump` (`:954`) each take an explicit `now_ms`; there is no clock field and no timer.
- `monotonic time` (`crates/crpg-server/src/host.rs:679`) — `check_time` rejects a `now_ms` earlier than the last accepted one with `HostError::TimeRegression` (`:327`).
- `sim time is separate` (`crates/crpg-server/src/host.rs:630`) — `server_tick()` reads `history.world().tick().get()`; `tick()` (`:1950`) is the only advance, and it is a trusted explicit call, not a loop.
- `server source has no thread, sleep or clock` (`crates/crpg-server/AGENTS.md:69`) — invariant 7: "`now_ms` only; no clock, thread, sleep or socket in `crpg-server` source."
- `adapter spawns nothing` (`crates/crpg-server/src/quic.rs:21`) — "This module reads no clock, spawns no thread, opens no socket and never pauses: `now_ms` is injected by the caller, and the socket, runtime and I/O thread belong to the `QuicServer`."
- `blocking calls are the caller's` (`crates/crpg-server/src/quic.rs:24`) — `wait` (`:1160`), `shutdown` (`:1403`) and dropping the adapter are the only blocking operations, and only the caller makes them.
- `pump cadence constraint` (`crates/crpg-server/AGENTS.md:226`) — "The pump cadence must beat `decision_timeout_ms` (5 s)"; "Pump at most every 50 ms, or after `wait` returns."
- `net-quic owns the I/O thread` (`crates/crpg-net-quic/src/lib.rs:14`) — each `QuicServer` and `QuicClient` owns one UDP socket, one OS thread and one current-thread tokio runtime.
- `net-quic thread spawn` (`crates/crpg-net-quic/src/server.rs:269`) — `std::thread::Builder::new().spawn(...)`; `server_main` is `async` (`:834`) and each connection gets `tokio::spawn` (`:892`).
- `net-quic deadlines` (`crates/crpg-net-quic/src/queue.rs:59`) — `deadline_after(ms)` computes `Instant::now() + ms` for this crate's queue deadlines; the host sees only `now_ms` values it passes in.
- `net fabric has no time source of its own` (`crates/crpg-net/src/sim.rs:15`) — "No sleep, no threads, no I/O, no wall clock: time enters only through `SimClock`".
- `exact-build determinism` (`AGENTS.md:20`) — replay determinism is exact-build, not cross-platform lockstep; each target compares its own golden; no tolerance and no missing-baseline skips.
- `determinism bans` (`AGENTS.md:43`) — no `HashMap`/`HashSet` in `crpg-core`, `crpg-rules`, `crpg-sim`; no `f32`/`f64` in core/rules (`:45`); `f64` is banned in sim as well (`:47`).
- `IO-free lower crates` (`tools/lint/deps.py:81`) — `IO_FREE_CRATES = {crpg-core, crpg-data, crpg-rules, crpg-sim, crpg-net}`; a direct tokio/quinn/rustls entry in any of them is a violation (`:82`).
- `CI executes workspace tests on both targets` (`.github/workflows/ci.yml:159`) — `cargo test --workspace --locked` on `ubuntu-latest` and `windows-latest`; this fact-sheet task changes only `tasks/`, so the docs-only skip (`:138`) applies.

## Toolchain

- `Godot4 --version` (this machine, 2026-11-01) — output is exactly `4.7.2.stable.official.ed1daf0bf`.
- `godot crate in the local cargo cache` (`C:\Users\jwgel\.cargo\registry\cache\index.crates.io-1949cf8c6b5b557f\godot-0.5.5.crate`) — the cache holds `godot` and its `godot-bindings`, `godot-cell`, `godot-codegen`, `godot-core`, `godot-ffi`, `godot-macros` companions, all at `0.5.5`.
- `godot versions in the local index cache` (`C:\Users\jwgel\.cargo\registry\index\index.crates.io-1949cf8c6b5b557f\.cache\go\do\godot`) — the newest of `0.4.4, 0.4.5, 0.5.0, 0.5.1, 0.5.2, 0.5.3, 0.5.4, 0.5.5` is `0.5.5`, none yanked; this is a local cache and does not prove the current crates.io stable.
- `D27d recorded pin` (`tasks/DECISIONS-2026-10-04.md:69`) — "0.5.5 on 2026-10-04", exact pin, `crpg-godot` only.
- `workspace has no godot pin` (`crates/crpg-godot/Cargo.toml:7`) — the bridge crate's `[dependencies]` is empty, so no `godot` version is pinned anywhere in the workspace or `Cargo.lock`.
- `Godot 4.x line` (`docs/CRPG_ENGINE_SPEC.md:1725`) — technical decision 5: "Latest stable 4.x at Phase 0, pinned by tag; upgrade deliberately once per minor release".

## Agent log

- 2026-11-01 (UTC) · opencode-go/deepseek-v4.1-flash + T045 facts · Gathered the bridge requirements, earlier spike measurements, as-built `crpg-godot`/lint/CI state, host surface, client data, threading and toolchain with verifiable `path:line` citations, so the T045 ADR and T047 contract can be written from facts rather than a re-read of the tree.
