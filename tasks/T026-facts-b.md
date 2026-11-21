# T026 facts B — Godot client side

## Bridge today

- `CrpgSession` (`crates/crpg-godot/src/shell.rs:29`): `#[derive(GodotClass)] #[class(base = RefCounted, init)]` struct holding `core: BridgeCore` and `opened_at: Option<Instant>`.
- `CrpgView` (`crates/crpg-godot/src/shell.rs:139`): `#[derive(GodotClass)] #[class(base = RefCounted, no_init)]` struct wrapping `view: Rc<SessionView>`.
- `CrpgExtension` (`crates/crpg-godot/src/shell.rs:17`) with the crate's only hand-written `unsafe impl ExtensionLibrary` (`crates/crpg-godot/src/shell.rs:26`) under a `SAFETY:` comment block (`crates/crpg-godot/src/shell.rs:19-24`).
- `CrpgSession` `#[func]` methods: `open` (`crates/crpg-godot/src/shell.rs:39`), `open_v4` (`crates/crpg-godot/src/shell.rs:62`), `declare_move` (`crates/crpg-godot/src/shell.rs:84`), `move_to` (`crates/crpg-godot/src/shell.rs:90`), `halt` (`crates/crpg-godot/src/shell.rs:96`), `poll` (`crates/crpg-godot/src/shell.rs:102`), `view` (`crates/crpg-godot/src/shell.rs:110`), `submit_option` (`crates/crpg-godot/src/shell.rs:118`), `close` (`crates/crpg-godot/src/shell.rs:124`), `last_error` (`crates/crpg-godot/src/shell.rs:130`).
- `BridgeCore` (`crates/crpg-godot/src/core/mod.rs:50`) holds `state: State`, `view: Rc<SessionView>`, `last_error: String`; states are `Closed`/`Open(Box<Live>)`/`OpenV4(Box<LiveV4>)`/`Fenced(BridgeCode)` (`crates/crpg-godot/src/core/mod.rs:59-71`).
- Embedded host start (v2): `Host::from_history` with `ProtocolSelection::V2` (`crates/crpg-godot/src/core/mod.rs:342-349`); player peer via `host.bind_snapshot_peer` (`crates/crpg-godot/src/core/mod.rs:353-362`); NPC peer via `host.bind_peer` (`crates/crpg-godot/src/core/mod.rs:363-372`).
- Embedded host start (v4): `Host::from_history` with `ProtocolSelection::V2` (`crates/crpg-godot/src/core/mod.rs:440-449`), `host.bind_area_bounds` (`crates/crpg-godot/src/core/mod.rs:451-453`), player peer via `host.bind_v4_peer` (`crates/crpg-godot/src/core/mod.rs:454-465`), NPC peer via `host.bind_peer` (`crates/crpg-godot/src/core/mod.rs:466-475`), then `LocalHost::new(host)` (`crates/crpg-godot/src/core/mod.rs:491`), `local.attach(player, NonZeroUsize::new(LINK_FRAMES)...)` (`crates/crpg-godot/src/core/mod.rs:492-495`), `spawn(HostLoop::new(local, 0), clock)` (`crates/crpg-godot/src/core/mod.rs:497-500`).
- Protocol labels: ADR-0033 §8 assigns the embedded player session the `/2` snapshot-peer kind (`docs/adr/0033-godot-bridge.md:227-228`); `crpg-lane0/1`, `/2`, `/3` connections bind via `Host::bind_peer`, `bind_snapshot_peer`, `bind_v4_peer` respectively (`crates/crpg-server/src/quic.rs:474`, `crates/crpg-server/src/quic.rs:484`, `crates/crpg-server/src/quic.rs:587`).
- Replica reads: `ClientV4::step` routes `0x82` transfers to `ReassemblyPoolV4`, `0xA1` lane-1 datagrams to `decode_down`, else deltas (`crates/crpg-godot/src/core/client_v4.rs:99-148`); position state is read via `self.replica.position(entity.net_id)` (`crates/crpg-godot/src/core/client_v4.rs:456`), whose method is `ReplicaWorldV4::position` (`crates/crpg-net/src/replica_v4.rs:77-79`).
- Core/Godot split: crate root declares `pub mod core;` and `#[cfg(feature = "engine")] mod shell;` (`crates/crpg-godot/src/lib.rs:12-15`); `core/mod.rs` begins with `#![forbid(unsafe_code)]` (`crates/crpg-godot/src/core/mod.rs:1`); shell imports `crate::core::{BridgeCore, SessionView, ViewValue}` (`crates/crpg-godot/src/shell.rs:14`).
- Test helpers in `crates/crpg-godot/src/core/authority/v4/tests.rs`: `ManualClock` struct (`crates/crpg-godot/src/core/authority/v4/tests.rs:30`), `ManualClock::new` (`crates/crpg-godot/src/core/authority/v4/tests.rs:35`), `ManualClock::set` (`crates/crpg-godot/src/core/authority/v4/tests.rs:41`), `call_result` (`crates/crpg-godot/src/core/authority/v4/tests.rs:253`), `call` (`crates/crpg-godot/src/core/authority/v4/tests.rs:262`), `settle` (`crates/crpg-godot/src/core/authority/v4/tests.rs:295`).

## Positions in the client today

- `CrpgView::positions` (`crates/crpg-godot/src/shell.rs:231-235`), `position_known` (`crates/crpg-godot/src/shell.rs:237-241`) and `moving` (`crates/crpg-godot/src/shell.rs:243-247`) expose position data to GDScript.
- `SessionView` carries `positions: Vec<i64>` and `position_known: Vec<u8>` (`crates/crpg-godot/src/core/client.rs:45-46`), with accessors `positions()` (`crates/crpg-godot/src/core/client.rs:133-135`) and `position_known()` (`crates/crpg-godot/src/core/client.rs:138-140`).
- `ClientV4::rebuild` fills `positions`, `position_known`, `moving` from `self.replica.position(entity.net_id)` (`crates/crpg-godot/src/core/client_v4.rs:452-468`).
- v2 `Client` zeroes the position columns: `positions: vec![0; 3 * rows]`, `position_known: vec![0; rows]` (`crates/crpg-godot/src/core/client.rs:600-601`).
- `ReplicaWorldV4` stores disclosed positions in `positions: BTreeMap<NetId, PositionState>` (`crates/crpg-net/src/replica_v4.rs:57`); `PositionState` has `tick`, `point: Point`, `moving` (`crates/crpg-net/src/replica_v4.rs:32-39`); `position()` returns `Option<&PositionState>` (`crates/crpg-net/src/replica_v4.rs:77-79`); apply_op handles `DeltaOp::Position` and `EntityLeave`/`Despawned` removes the key (`crates/crpg-net/src/replica_v4.rs:175-213`).
- `SceneSync.sync` sets `proxy.position = Vector3(3.0 * float(side_index), 0.0, 1.5 * float(rank))` (`apps/client/scripts/SceneSync.gd:61`).
- `SceneSync._make_proxy` sets `label.position = Vector3(0.0, 1.2, 0.0)` (`apps/client/scripts/SceneSync.gd:89`).
- `SceneSync._set_mesh` sets `mesh_instance.position.y = mesh_height / 2.0` (`apps/client/scripts/SceneSync.gd:122`).
- `Main.tscn` has `Camera3D` transform `Transform3D(1, 0, 0, 0, 0.866025, 0.5, 0, -0.5, 0.866025, 0, 6, 9)` (`apps/client/scenes/Main.tscn:20-21`) and `Sun` transform `Transform3D(0.707107, 0.5, -0.5, 0, 0.707107, 0.707107, 0.707107, -0.5, 0.5, 0, 5, 0)` (`apps/client/scenes/Main.tscn:23-24`).

## Scripts and scenes

- `apps/client/scenes/Main.tscn` declares `Main` (`Node3D` with `Main.gd`), `Camera3D`, `Sun` (`DirectionalLight3D`), `Environment` (`WorldEnvironment`), `Ground` (`MeshInstance3D`), `Proxies` (`Node3D` with `SceneSync.gd`) and `Hud` (`CanvasLayer` with `CombatHud.gd`) (`apps/client/scenes/Main.tscn:17-36`).
- `Main.gd` owns `session`, `seen`, `view`, `open_code`, `stop_code` (`apps/client/scripts/Main.gd:7-11`); `_ready` instantiates `CrpgSession` and calls `open(_campaign_dir(), "", "", OPEN_SEED)` with `OPEN_SEED := 1` (`apps/client/scripts/Main.gd:5,13-25`); `_process` calls `session.poll()` once, treats code 30 as a notice, sets `stop_code` on other non-zero, and on a revision change calls `Proxies.sync(view)` and `Hud.sync(view)` (`apps/client/scripts/Main.gd:26-44`); `choose(index)` calls `session.submit_option(seen, index)` (`apps/client/scripts/Main.gd:45-49`); `_exit_tree` calls `session.close()` (`apps/client/scripts/Main.gd:50-53`).
- `SceneSync.gd` holds `_proxies: Dictionary` and `_side_order: Array[String]` (`apps/client/scripts/SceneSync.gd:14-15`); `plan(ids)` returns `{"despawn", "spawn", "refresh"}` ascending lists (`apps/client/scripts/SceneSync.gd:17-37`); `sync(view)` frees despawns with `remove_child` then `queue_free`, spawns, then rewrites position, mesh, name, dead tint and `move_child(proxy, i)` (`apps/client/scripts/SceneSync.gd:39-67`); `clear()` frees all and clears `_side_order` (`apps/client/scripts/SceneSync.gd:68-72`); `proxy_count()` counts `proxy_` children (`apps/client/scripts/SceneSync.gd:73-78`); `_make_proxy` creates a `Node3D` with a `Mesh` `MeshInstance3D` and a billboarded `Label` `Label3D` at `y = 1.2` (`apps/client/scripts/SceneSync.gd:80-97`).
- `CombatHud.gd` builds its named node tree in `_init` (`apps/client/scripts/CombatHud.gd:24-59`) with `main = get_parent()` in `_ready` (`apps/client/scripts/CombatHud.gd:60-61`); `sync(view)` writes round/turn/health/options/waiting/log/end (`apps/client/scripts/CombatHud.gd:63-75`); `show_error`/`show_notice`/`clear` are defined at `apps/client/scripts/CombatHud.gd:76-95`.
- Input handling: none in the client today; `docs/architecture/crpg-godot.md:259-260` states "No input. Camera control and picking are T050 and deliberately absent; T049 owns positions."
- `load.gd` (`apps/client/tests/load.gd`): exits 0 pass, 2 class missing, 3 first failed check (`apps/client/tests/load.gd:3-5`); control with missing campaign exits 3 (`apps/client/tests/load.gd:5`).
- `scene_sync.gd` runner (`apps/client/tests/scene_sync.gd`): exits 0 on `"PASS"`, 6 on any other result, 7 on deferred-free check failure, 3 on script load failure (`apps/client/tests/scene_sync.gd:4-6`).
- `scene_sync_cases.gd` holds S1-S13 and S15-S19 (T048/T048b/T048c) and S14 is checked two frames later by the runner (`apps/client/tests/scene_sync.gd:2`, `apps/client/tests/scene_sync.gd:37-48`).
- `hud.gd` runner (`apps/client/tests/hud.gd`): exits 0 on `"PASS"`, 6 on any other result, 3 on load failure (`apps/client/tests/hud.gd:2-4`); `hud_cases.gd` holds H1-H19.
- `smoke.gd` (`apps/client/tests/smoke.gd`): runs scene-sync then HUD cases first, exits 6/7 on their failure, then plays `combat_basic` choosing option 0, exits 0 when ended with `proxy_count() == net_ids().size() == 2`, 5 on proxy-count mismatch, 3 on open/poll/choose failure, 2 on missing class, 4 after 120 s (`apps/client/tests/smoke.gd:4-8,68-94`).
- `movement.gd` (`apps/client/tests/movement.gd`): exits 0 pass, 2 class missing, 3 unexpected code, 4 timeout, 5 MOVE edit/copy failure, 6 value check failure (`apps/client/tests/movement.gd:9-10`); runs M1-M7 (`apps/client/tests/movement.gd:123-141`).

## Rules that bind it

- `crates/crpg-godot/AGENTS.md` invariants: core is Godot-free with `#![forbid(unsafe_code)]` in `core/mod.rs` (`crates/crpg-godot/AGENTS.md:12-15`); only `shell.rs` carries the one hand-written `unsafe` (`crates/crpg-godot/AGENTS.md:56-60`); `godot` is optional behind `engine` (off by default) and pinned `=0.5.5` (`crates/crpg-godot/AGENTS.md:59-60`); engine gates never run under `cargo test` (`crates/crpg-godot/AGENTS.md:90`); `crpg-godot` is outside `determinism.py` (`docs/adr/0033-godot-bridge.md:371`).
- Threading/frame loop: bridge never ticks, pumps or drives lane 1 (`crates/crpg-godot/AGENTS.md:69-73`); `poll` never waits on the loop thread, only `close` and a fence `call` do (`crates/crpg-godot/AGENTS.md:74-78`); one thread, no spawn (`docs/adr/0033-godot-bridge.md:111-115`); `shell.rs` reads `Instant` and passes elapsed ms to `poll` (`docs/adr/0033-godot-bridge.md:117-126`); per-frame flow is `Main.gd _process` → `session.poll()` → authority step → client step → view (`docs/adr/0033-godot-bridge.md:128-157`).
- ADR-0033 FFI surface: two `RefCounted` classes `CrpgSession` and `CrpgView` with the member table (`docs/adr/0033-godot-bridge.md:251-287`); no signals (`docs/adr/0033-godot-bridge.md:257-258`).
- ADR-0033 embedded host: `Host::from_history` with `ProtocolSelection::V2`, `incarnation: 1`, bind snapshot peer for player and plain peer for NPC (`docs/adr/0033-godot-bridge.md:67-94`).
- ADR-0033 error mapping: closed code table 0-55 with fatal exactly 50-55 (`docs/adr/0033-godot-bridge.md:316-349`).
- ALLOWED row for `crpg-godot`: `"crpg-godot": (None, set())` (`tools/lint/deps.py:71`); `GODOT_CRATE = "crpg-godot"` (`tools/lint/deps.py:75`).
- `docs/IMPLEMENTATION_PLAN.md` §1.4 lists "the GDExtension boundary in `crpg-godot`" among serialized items (`docs/IMPLEMENTATION_PLAN.md:95`); packages marked **S** in §3 are in that set (`docs/IMPLEMENTATION_PLAN.md:98`); T047 is the only crpg-godot task marked S (`docs/IMPLEMENTATION_PLAN.md:259`). The boundary surfaces are `crates/crpg-godot/src/shell.rs` (the `#[gdextension] unsafe impl`, `crates/crpg-godot/src/shell.rs:25-26`), `crates/crpg-godot/src/lib.rs` (`#![allow(unsafe_code)]` at `crates/crpg-godot/src/lib.rs:1`), and `apps/client/crpg.gdextension` (`entry_symbol = "gdext_rust_init"`, `apps/client/crpg.gdextension:2`).

## Player UX research

- Click-to-move: none found in `docs/research/T104/` or `docs/research/T107/`.
- Movement feedback: none found in `docs/research/T104/` or `docs/research/T107/`.
- Path preview: none found in `docs/research/T104/` or `docs/research/T107/`.
- Selection: none found in `docs/research/T104/` or `docs/research/T107/`.
- Camera: `docs/research/T104/R16-deep.md:15-18` recommends a free-floating camera (isometric to third-person) plus tactical zoom; `docs/research/T104/R16-deep.md:54` lists Baldur's Gate 3 "Free-floating; isometric to third-person"; `docs/research/T104/R16-deep.md:62` lists XCOM 2 "Top-down".
- Targeting preview (adjacent): `docs/research/T104/R16-deep.md:52` comparison-table header includes "Targeting preview"; `docs/research/T104/R16-deep.md:54` "Hit-chance % before attack"; `docs/research/T104/R16-deep.md:62` "Shot success % and health loss shown".
- `docs/research/T107/R18-deep.md` contains no matches for click, camera, selection, path preview or movement feedback.

## CI and run

## Agent log

- 2026-11-21 (UTC) · opencode-go/beam-free + T026 facts B · Fact sheet B for the Godot client side, citations only.