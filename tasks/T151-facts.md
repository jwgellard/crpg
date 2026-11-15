# T151 facts — the `crpg-godot` bridge's move to protocol v4

Fact sheet for T151, the bridge move owed by T026p and T037. Facts only: each
entry is a `path:line` citation and one line of what it says or does today.
`tasks/T037.md` is an approved contract for code not yet written; its items are
cited as the future surface and are marked as such.

## Requirements

Binding sentences, quoted. `tasks/T037.md` is an approved contract for code not
yet written; its items are cited as the future surface and marked so.

**`docs/adr/0033-godot-bridge.md`**

- `One crate, three parts` (`docs/adr/0033-godot-bridge.md:48`) — "The bridge is `crpg-godot` only: no new package, no `ALLOWED` change"; the module list names `core/client.rs` as the "untrusted half: replica, intent encoding, view building. Bytes only" (`:58`).
- `client.rs` source rule (`docs/adr/0033-godot-bridge.md:64`) — "`client.rs` names neither `crpg_server` nor `crpg_sim` (BR-10)"; "That is the compile-visible form of 'the client cannot mutate authoritative state'."
- `LocalLink` (`docs/adr/0033-godot-bridge.md:95`) — the transport is "a pair of bounded in-process FIFO byte queues implementing `crpg_net::transport::Transport`", "each direction capped at 128 frames".
- `Link carries lane 0 only` (`docs/adr/0033-godot-bridge.md:108`) — "The link carries lane-0 frames only. It needs no channel tag because a `DeliveryKind::Transfer` entry is a fatal fence in this slice (decision 8)."
- `Per-frame flow` (`docs/adr/0033-godot-bridge.md:130`) — "GDScript calls `session.poll()` exactly once per `_process`"; the client step is "`recv from link -> codec_v2::decode_delta -> apply to replica (T046)`" (`:152`).
- `Session kind` (`docs/adr/0033-godot-bridge.md:227`) — "The player binds with `bind_snapshot_peer`: same first delivery as `bind_peer`, and the session kind `/2` QUIC peers get"; "nothing sends `Resync`, so a `Transfer` entry is unreachable" (`:229–230`).
- `NetId`-only FFI (`docs/adr/0033-godot-bridge.md:236`) — "`EntityId` never crosses into the client half or the FFI; every id GDScript sees is a `NetId`."
- `FFI surface` (`docs/adr/0033-godot-bridge.md:251`) — "Two classes, both `RefCounted`, both prefixed"; "There are **no signals** in the slice" (`:256–257`).
- `Submission is by option index only` (`docs/adr/0033-godot-bridge.md:304`) — "GDScript cannot name an ability, target or actor, and never sees wire bytes, a seq or an epoch."
- `Error mapping` (`docs/adr/0033-godot-bridge.md:318`) — "Codes are stable integers in one closed table, `0` being success"; "Fatal means: the core calls `Host::shutdown`, drops the host, and every later `poll` returns the same code until `close`" (`:346–347`).
- `apps/client layout` (`docs/adr/0033-godot-bridge.md:423–424`) — "Proxy layout is client-only (one row per side), because the wire has no position. It decides nothing."
- `T049` (`docs/adr/0033-godot-bridge.md:519`) — "T049 is out of the slice and blocked on the lane-1 movement channel: there is nothing to interpolate. It later adds `positions()` and `previous_positions()` to `CrpgView`; adding to the view object is why this is an object."
- `v4` (`docs/adr/0033-godot-bridge.md`) — none found: the file has no `v4`, `ReplicaWorldV4` or `MoveOutbox`.

**`docs/adr/0036-movement-and-navigation.md`**

- `Context: gated client tasks` (`docs/adr/0036-movement-and-navigation.md:13–15`) — T034 "gates T035 (`crpg-nav` v1), T036 (authoritative movement, S-set), T026p (lane-1 movement, S-set), T037 (host adapter) and the client tasks T049, T026 and T050."
- `Context: identical navmesh` (`docs/adr/0036-movement-and-navigation.md:46`) — "Spec §6.3: server and client must use the identical navmesh".
- `A1` (`docs/adr/0036-movement-and-navigation.md:64`) — "Clients send destinations, never paths or positions." "There is no client-supplied path, speed or position (spec §7.3)."
- `A5` (`docs/adr/0036-movement-and-navigation.md:85`) — "The client's prediction and previews call them (`crpg-godot` may import `crpg-sim`), so client and server compute the same numbers."
- `L2` (`docs/adr/0036-movement-and-navigation.md:392`) — "Upstream is a reliable ordered stream: `MoveTo { seq, actor, destination: [i32; 3] }` … and `Halt { seq, actor }`. There are no other fields (A1)."
- `L3` (`docs/adr/0036-movement-and-navigation.md:398`) — "Every admitted intent gets exactly one receipt, in `seq` order."
- `L4` (`docs/adr/0036-movement-and-navigation.md:402`) — "Samples are unreliable datagrams: `MoveSample { server_tick, processed_seq, entries: […] }`"; "A client ignores any sample not newer than the newest it holds for that entity. Positions are integers, never floats."
- `L5` (`docs/adr/0036-movement-and-navigation.md:409`) — "Position is a per-field grant (T027b, H-filter). Samples and resync snapshots carry only granted positions, and revocation stops them at once."
- `L6` (`docs/adr/0036-movement-and-navigation.md:413`) — "TB moves ride lane 0 as a combat verb `DeclareMove { actor, destination }` … The result is a `Moved { net_id, points }` projection of `HistoryEvent::Moved`."
- `H4` (`docs/adr/0036-movement-and-navigation.md:436`) — "Each tick, send the granted entities that moved, plus every granted position at a refresh interval that T037 pins."
- `G1` (`docs/adr/0036-movement-and-navigation.md:446`) — "Each entity has a sample buffer keyed by server tick. Render at the newest tick minus the U7 delay, interpolating linearly between the two samples around it, after `to_f32`. With no newer sample, hold the last position; never extrapolate."
- `G2` (`docs/adr/0036-movement-and-navigation.md:453`) — "Only for actors the peer controls, and never in TB … Prediction runs only when the client's own baked mesh revision equals the announced one."
- `G3` (`docs/adr/0036-movement-and-navigation.md:460`) — "Pick a ground point and pass it through the same `snap` as the host (S3); preview the path and TB cost with local navigation … send `MoveTo` or `DeclareMove` by mode; never send a path. The `crpg-godot` to `crpg-nav` path edge needs approval."
- `Follow-on table` (`docs/adr/0036-movement-and-navigation.md:542`) — "`T049, T026, T050` | `crpg-godot` | G1, G2, G3 (with S3) | as the plan lists, plus T037".
- `ReplicaWorldV4 / MoveOutbox` (`docs/adr/0036-movement-and-navigation.md`) — none found: the ADR names the wire types, not the `crpg-net` Rust types.

**`docs/adr/0001-godot-pinned-not-forked.md`**

- `Pinned, not forked` (`docs/adr/0001-godot-pinned-not-forked.md:13`) — "Use Godot as an unmodified, version-pinned presentation host, consumed through GDExtension. Keep the entire simulation in Rust crates with no Godot dependency."
- `Cost: no Godot types in the core` (`docs/adr/0001-godot-pinned-not-forked.md:22`) — "Godot types must be kept out of the core, enforced by CI lint."

**`docs/IMPLEMENTATION_PLAN.md`**

- `T026` (`docs/IMPLEMENTATION_PLAN.md:257`) — "`T026` | C → I | `crpg-godot` | T026p, T037, T049 | Own-movement prediction and reconciliation".
- `T049` (`docs/IMPLEMENTATION_PLAN.md:256`) — "`T049` | C → I | `crpg-godot` | T047 | Interpolation buffer (§19.1 #23)".
- `T050` (`docs/IMPLEMENTATION_PLAN.md:258`) — "`T050` | C → I | `crpg-godot` | T048, T104 | Camera, click-to-move, selection".
- `T037` (`docs/IMPLEMENTATION_PLAN.md:225`) — "`T037` | C → I | crpg-server | T026p, T035, T036c | Movement host adapter. Split into T037a–e, serial (D60)".

**`tasks/T026p.md`**

- `Out of scope: crpg-godot` (`tasks/T026p.md:1477`) — "**`crpg-godot` (manager to file, after T037):** moving the bridge to `ReassemblyPoolV4`, `ReplicaWorldV4` and `MoveOutbox`; T049 interpolation (G1), T026 prediction (G2), T050 click-to-move (G3)."
- `Reading 12: MoveOutbox` (`tasks/T026p.md:287`) — "The client's lane-1 sender state lives in `crpg-net`. `MoveOutbox` holds the lane-1 rules from the client side … so `crpg-godot` (T026, T050) and the host's tests share one tested implementation."
- `Reading 9: Moved is unchecked` (`tasks/T026p.md:260`) — "**In the replica `Moved` is an unchecked notice** … the replica stores nothing from it. A presenter pairs the id with `v3().entity()` and treats a miss as 'not visible'. Positions come only from anchors and samples; G1's animation … is T049's."
- `Errors: no new ReplicaError` (`tasks/T026p.md:1042`) — "`FactError` …, `ProjectionError` and `ReplicaError` … are reused unchanged; no variant is added, so the bridge's fatal-53 mapping and every `Display` text stay."
- `Manager edits` (`tasks/T026p.md:1565`) — "the new `crpg-net-quic` task and the `crpg-godot` bridge task (Out of scope)" remain to be filed by the manager.

**`tasks/T037.md`** (approved contract, not yet implemented)

- `Out of scope: the bridge to v4` (`tasks/T037.md:1356`) — "**The `crpg-godot` bridge to v4** (manager files, after T037): the bridge's move to `ReassemblyPoolV4`, `ReplicaWorldV4` and `MoveOutbox`, and T049 (G1), T026 prediction (G2), T050 click-to-move (G3)."
- `Binding a v4 session` (`tasks/T037.md:439`) — future `Host::bind_v4_peer(control, grants, facts, pause_rights, now_ms) -> Result<PeerHandle, HostError>`; `NotV4Peer` is the new error for a call that needs such a session (`:434`).
- `Positions and withholding` (`tasks/T037.md:420`) — future `position_rows(world, grants, withheld) -> Vec<PositionRow>` and `set_position_withheld(peer, entities)` (`:443`).
- `DeclareMove` (`tasks/T037.md:1301`) — future test K2: `DeclareMove { P(32768, 0, 32768) }` by the hero is `Applied`, and the page carries `Moved { hero, [P(0,0,0), P(32768,0,32768)] }` (`:1302`).
- `Samples` (`tasks/T037.md:456`) — future `POLICY_SAMPLE_REFRESH_TICKS: u64 = 20` and `Host::take_samples(peer) -> Result<Vec<Vec<u8>>, HostError>` (`:462`).
- `Lane-1 ingest and receipts` (`tasks/T037.md:483`) — future `ingest_move(peer, bytes, now_ms) -> Result<MoveAdmission, HostError>` and `take_move_receipts(peer) -> Result<Vec<Vec<u8>>, HostError>` (`:487`).
- `What must not change: crpg-godot` (`tasks/T037.md:1412`) — "`cargo check --workspace` proves `crpg-godot` (which maps `HostError` with a wildcard, `crates/crpg-godot/src/core/errors.rs:106–110`, and matches `DeliveryKind` exhaustively, `crates/crpg-godot/src/core/authority.rs:61–75`) compiles unchanged."

## The bridge today

(section pending)

## The v4 client types in crpg-net

(section pending)

## The host side the bridge talks to

(section pending)

## The Godot client

(section pending)

## Existing tests to copy

(section pending)

## Rules that bind it

(section pending)

## Agent log

- 2026-11-15 (UTC) · deepseek-v4.1-flash + T151 facts · Skeleton with the
  required section headings, so each section lands as its own reviewed commit.
