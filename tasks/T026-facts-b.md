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

## Rules that bind it

## Player UX research

## CI and run

## Agent log

- 2026-11-21 (UTC) · opencode-go/beam-free + T026 facts B · Fact sheet B for the Godot client side, citations only.