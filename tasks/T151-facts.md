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

**Modules in `crates/crpg-godot/src/`**

- `lib.rs` (`crates/crpg-godot/src/lib.rs:1`) — crate root: `#![allow(unsafe_code)]` for macro expansion, `#![warn(missing_docs)]`, `pub mod core` (`:12`), and `#[cfg(feature = "engine")] mod shell` (`:14–15`).
- `core/mod.rs` (`crates/crpg-godot/src/core/mod.rs:1`) — `#![forbid(unsafe_code)]`; private `authority`, `client`, `errors`, `link` (`:8–11`); re-exports `SessionView`, `ViewEvent`, `ViewReceipt`, `ViewValue`, `BridgeCode` (`:33–34`); `BridgeCore` holds `State { Closed, Open(Box<Live>), Fenced }` (`:48–56`) and `Live { authority, client, link }` (`:59–65`).
- `core/errors.rs` (`crates/crpg-godot/src/core/errors.rs:1`) — the closed `BridgeCode` table (`:12–53`), `name()` (`:62`), `is_fatal()` exactly `50..=55` (`:88–90`), `Fault` (`:95`), and `host_fault` (`:106`).
- `core/link.rs` (`crates/crpg-godot/src/core/link.rs:1`) — `LocalLink` is two `Rc<RefCell<VecDeque<Vec<u8>>>>` queues (`:19–26`), `LINK_FRAMES = POLICY_EGRESS_FRAMES_PER_PEER` (`:17`), `authority_end`/`client_end` (`:50`, `:59`), and `Transport` for `LinkEnd` where a full queue is `LinkFull` and changes nothing (`:77–84`).
- `core/client.rs` (`crates/crpg-godot/src/core/client.rs:1`) — `SessionView` with 16 members (`:28–45`), `Client::new` builds `ReplicaWorld::new(epoch)` (`:292`), `step` drains the link, decodes and applies, collects events/receipts (`:313–330`), `submit` encodes one intent (`:491`, `:523`).
- `core/authority.rs` (`crates/crpg-godot/src/core/authority.rs:1`) — the campaign walk (`WalkFs`/`RealFs`/`collect_campaign`), `check_binding`, `Presentation`, `Authority` (`:462–474`), `hand_off` (`:55`), `npc_results` (`:83`), `choose_npc` (`:34`), `NpcPacer` (`:112`), and the open helpers.
- `core/tests.rs` and `core/{authority,client,errors,link}/tests.rs` (`crates/crpg-godot/src/core/tests.rs:1`) — `#[cfg(test)]` suites: 21 cases in `core/tests.rs`, 7 in `authority/tests.rs`, 12 in `client/tests.rs`, 2 in `errors/tests.rs`, 3 in `link/tests.rs`.
- `shell.rs` (`crates/crpg-godot/src/shell.rs:1`) — the gdext shell, compiled only with `engine`; `unsafe impl ExtensionLibrary` at `:26` under the `SAFETY:` block at `:19–24`.

**How `link.rs`, `client.rs` and `authority.rs` use a reassembly pool and replica world**

- `core/link.rs` (`crates/crpg-godot/src/core/link.rs:22–33`) — no reassembly pool and no replica: it stores whole `Vec<u8>` frames in two FIFOs and implements only `Transport`; `recv` returns `Option<Vec<u8>>` (`:86–88`).
- `ReassemblyPool*` in `crates/crpg-godot` — none found: a search for `Reassembly`, `Pool` and `pool` over the crate returns no match, because a `DeliveryKind::Transfer` entry is a fatal fence instead (`crates/crpg-godot/src/core/authority.rs:62–67`).
- `core/client.rs` (`crates/crpg-godot/src/core/client.rs:18`) — the client half holds one `crpg_net::replica::ReplicaWorld` (the v2 replica, `crates/crpg-net/src/replica.rs:121`), built at `:292`; it decodes with `crpg_net::codec_v2::decode_delta` (`:12`, `:322`), applies with `ReplicaWorld::apply_delta` (`:326`), and reads `entities()`/`turn()` to rebuild the view (`:398–438`). It submits with `codec_v2::encode_intent` (`:12`, `:523`).
- `core/authority.rs` (`crates/crpg-godot/src/core/authority.rs:20–23`) — the trusted half owns the `Host`, both `PeerHandle`s and a `LinkEnd`; it calls `host.ingest` (`:513`, `:548`), `legal_intents` (`:525`, `:603`), `pump` (`:567`), `take_delivery_entries` (`:577`), `acknowledge_delivery` (`:581`, `:590`), `last_capture`/`acknowledge_captures` (`:595–597`).
- `ProtocolSelection::V2` (`crates/crpg-godot/src/core/mod.rs:302`) — `Host::from_history` is called with `HostConfig { protocol: ProtocolSelection::V2, incarnation: 1 }`, so every session the bridge opens today is a v2 session; the link therefore carries v2 deltas and v2 intents (`crates/crpg-godot/src/core/client.rs:12–18`).

**The `HostError` mapping**

- `host_fault` (`crates/crpg-godot/src/core/errors.rs:106–110`) — `HostError::Closed` maps to `BridgeCode::HostClosed` (50) and every other `HostError` to `HostFailed` (51) through the wildcard arm; `error.to_string()` becomes `Fault::detail`.

**The exhaustive `DeliveryKind` match**

- `hand_off` (`crates/crpg-godot/src/core/authority.rs:61–75`) — matches `DeliveryKind::Transfer` (fatal `UnexpectedTransfer` 54 with the ordinal in the detail) and `DeliveryKind::Delta` (sends; a `LinkFull` breaks the loop), with no wildcard arm, so a new variant is a compile error.

**GDExtension-exposed classes and their methods**

- `CrpgSession` (`crates/crpg-godot/src/shell.rs:29–34`) — `#[derive(GodotClass)]`, `#[class(base = RefCounted, init)]`, holding `core: BridgeCore` and `opened_at: Option<Instant>`.
- `CrpgSession` methods (`crates/crpg-godot/src/shell.rs:39–92`) — `#[func] open(campaign_dir, encounter_id, player_side, seed) -> i64` (`:39`), `poll() -> i64` (`:61`), `view() -> Gd<CrpgView>` (`:70`), `submit_option(revision, index) -> i64` (`:78`), `close()` (`:84`), `last_error() -> GString` (`:91`).
- `CrpgView` (`crates/crpg-godot/src/shell.rs:98–104`) — `#[derive(GodotClass)]` with `no_init`, wrapping one `Rc<SessionView>`.
- `CrpgView` methods (`crates/crpg-godot/src/shell.rs:107–202`) — 16 `#[func]` getters: `revision`, `net_ids`, `names`, `sides`, `placements`, `health`, `max_health`, `health_known`, `dead`, `active`, `round`, `ended`, `option_labels`, `option_targets`, `events`, `receipts` (the last two at `:191`/`:201`). No position getter exists.
- Classes present (`crates/crpg-godot/src/shell.rs:17`) — `CrpgExtension`; `ClassDB.class_exists("CrpgSession")` is the load proof used by the tests (`apps/client/tests/load.gd:18`).

**The `engine` feature**

- `engine = ["dep:godot"]` (`crates/crpg-godot/Cargo.toml:10–11`) — off by default; `godot = { version = "=0.5.5", optional = true, features = ["api-4-7"] }` (`:19`); `[lib] crate-type = ["cdylib", "rlib"]` (`:7–8`).

## The v4 client types in crpg-net

**`ReassemblyPoolV4`** (`crates/crpg-net/src/pool_v4.rs:102`)

- `new(limits: TransferLimitsV3) -> Result<Self, LimitsError>` (`:113`) — validates the v3 limits; `limits()` returns them (`:126`).
- `add_receiver(epoch: [u8; 16], start: GateMode) -> Result<ReceiverIdV4, TransferError>` (`:131`), `remove_receiver(id)` (`:165`), `rebind_receiver(id, now_ms) -> Result<Rebound, TransferError>` (`:723`).
- `offer_transfer(id, bytes, now_ms) -> Result<TransferOutcome, TransferError>` (`:227`) — accepts `0x82` v3 transfer frames for a v4 receiver; `offer_delta(id, bytes, now_ms) -> Result<DeltaOutcome, TransferError>` (`:534`) — decodes v4 delta bytes with `codec_v4::decode_delta` (`:22`, `:546`) and gates them by `event_seq` only (`:550`).
- What it yields: `pop_update(id) -> Option<ReplicaUpdateV4>` (`:656`), where `ReplicaUpdateV4` is `Snapshot(PublicationV3)` or `Delta(DeltaFrame)` (`:42–47`); `status(id) -> Result<ReceiverStatusV3, TransferError>` (`:673`); `poll(now_ms) -> Vec<ReceiverIdV4>` returns expired receivers (`:647`).
- v4 only: it "refuses v1, v2 and v3 delta bytes and `0x81` transfer frames with `Codec(UnsupportedVersion)`" (`:15–17`); it reads no op, so `Moved`/`Position` pass through unread (`:11–13`).

**`ReplicaWorldV4`** (`crates/crpg-net/src/replica_v4.rs:55`)

- `new(epoch: SessionEpoch)` (`:63`) — empty rows, positions and sample clock; `v3() -> &ReplicaWorldV3` (`:72`) answers every v3 query.
- `apply_delta(&DeltaFrame) -> Result<(), ReplicaError>` (`:96`) — checks lane, op count and `event_seq` (`:97–111`), applies on a clone and commits only on success (`:113–127`); `apply_snapshot(&PublicationV3)` (`:135`) empties positions (the next page re-anchors them) and keeps the sample clock.
- Positions: `position(entity: NetId) -> Option<&PositionState>` (`:77`) and `positions() -> &BTreeMap<NetId, PositionState>` ascending (`:83`); `PositionState { tick, point, moving }` (`:32–39`).
- Samples: `apply_sample(&MoveSample) -> usize` (`:148`) never fails, stores an entry only for an entity that already has a disclosed position and only when strictly newer (`:144–147`), and moves `sample_clock() -> Option<SampleClock>` (`:88`) only to a strictly newer `server_tick` (`:149–158`).
- `Moved` (`:186`) — an unchecked notice: "the replica stores nothing from it" (`:8`); a presenter pairs the id with `v3().entity()` and treats a miss as "not visible" (`tasks/T026p.md:266–267`).
- `DeltaOp::Position { entity, position }` (`:187–213`) — `Some(fact)` replaces unless the stored tick is newer; `None` removes the position; an unknown entity is `ReplicaError::UnknownEntity` (`:188–190`).

**`MoveOutbox`** (`crates/crpg-net/src/outbox_v4.rs:54`)

- `new(epoch: [u8; 16])` (`:63`) — next `seq` 1, nothing pending; `epoch()` (`:73`), `next_seq()` (`:78`), `processed_seq()` (`:83`), `pending() -> &[MoveIntent]` ascending, the resend list after a resume (`:88`).
- `send(actor: NetId, body: MoveBody) -> Result<MoveIntent, OutboxError>` (`:93`) — refuses `Exhausted` above `SEQ_LAST` and `Full` at `POLICY_MOVE_IN_FLIGHT` (8) (`:94–99`).
- `receive(&MoveReceipt) -> Result<(), OutboxError>` (`:113`) — completes only the oldest pending intent, whatever the outcome (`Accepted`, `Superseded`, `Rejected`), and checks epoch, `seq` and actor (`:114–131`); `OutboxError` has five variants (`:17–36`).
- Related lane-1 wire types: `MoveBody` (`crates/crpg-net/src/movement_v4.rs:76`), `MoveIntent { epoch, seq, actor, body }` (`:89`), `MoveOutcome` (`:102`), `MoveReceipt { epoch, seq, processed_tick, actor, outcome }` (`:116`), `MoveSample` (`:143`), `encode_intent`/`decode_intent` (`:207`/`:228`), `encode_down`/`decode_down` (`:273`/`:318`), `sample_datagrams` (`:447`).

**The v3 types they replace**

- `ReassemblyPoolV3` (`crates/crpg-net/src/pool_v3.rs:139`) — same gate for v3 deltas; `pool_v4.rs` is "`pool_v3` with the delta decoder and frame type swapped and its own id and update types", and reuses `PublicationV3`/`ReceiverStatusV3` (`tasks/T026p.md:271–275`).
- `ReplicaWorldV3` (`crates/crpg-net/src/replica_v3.rs:56`) — rows, `turn()`, `encounter()`, `timeline()`, `pause()`, `host_facts()` (`:92–133`) but no positions; `ReplicaWorldV4` wraps it and applies every `V3` op through its `apply_op` (`crates/crpg-net/src/replica_v4.rs:178–184`).
- `MoveOutbox` has no v3 counterpart: lane 1 is new in v4 (`crates/crpg-net/src/movement_v4.rs:32–34`).

**Differences from the types the bridge uses now**

- The bridge has no pool at all and decodes whole v2 frames itself (`crates/crpg-godot/src/core/client.rs:321–329`); a v4 client feeds `ReplicaUpdateV4` from `ReassemblyPoolV4::pop_update` and applies `Snapshot` via `apply_snapshot` / `Delta` via `apply_delta`.
- The bridge's replica is `crpg_net::replica::ReplicaWorld` (v2, `crates/crpg-net/src/replica.rs:121`), whose `apply_delta` takes `protocol_v2::DeltaFrame` (`crates/crpg-net/src/replica.rs:19`) and which has no `position`, `positions`, `sample_clock` or `apply_sample`.
- The bridge's `Client` keeps one lane-0 `pending: Option<u64>` and `next_seq` (`crates/crpg-godot/src/core/client.rs:296–297`); `MoveOutbox` is the separate lane-1 sequence state, with a window of 8 and receipts taken in `seq` order.
- `ReplicaWorldV4` exposes `Moved` only as the unchecked notice above; the bridge's view has no position member (`crates/crpg-godot/src/core/client.rs:28–45`), and the v1/v2 wire has "no transform op" (`crates/crpg-net/src/protocol.rs:305`).

## The host side the bridge talks to

**How the embedded single-player server is created from the bridge**

- `BridgeCore::open` (`crates/crpg-godot/src/core/mod.rs:99`) — builds `&LocalLink::new(LINK_FRAMES)` (`:111`; `LINK_FRAMES` is the host's per-peer egress depth, `crates/crpg-godot/src/core/link.rs:17`) and passes it into `open_on` (`:168`), so tests can inspect both queues.
- `Host::from_history` (`crates/crpg-godot/src/core/mod.rs:299–309`) — the authority is the `HistoryWorld` the campaign walk started, wrapped with `HostConfig { protocol: ProtocolSelection::V2, incarnation: 1 }` and `now_ms` 0; a failure is `StartFailed` (25).
- Peers (`crates/crpg-godot/src/core/mod.rs:325–338`) — the player binds with `bind_snapshot_peer(ControlGrant { actors }, disclosure, 0)` and the NPC with `bind_peer(ControlGrant { actors }, disclosure, 0)`; both feed the same `Host`.
- Wiring (`crates/crpg-godot/src/core/mod.rs:345–357`) — `host.epoch(player)` becomes the client replica's `SessionEpoch`, `Client::new(epoch, table)` (`:349`), then `Authority::new(host, player, npc, link.authority_end(), npc_sides, bound)` (`:350`) and `Live { authority, client, link: link.client_end() }` (`:351–357`). The two ends are the only channel between the halves.
- `Authority::step` (`crates/crpg-godot/src/core/authority.rs:508–607`) — drains the player's queue into `host.ingest` (`:512–519`), drives the NPC (`:522–548`), `host.pump` (`:567`), `take_delivery_entries` then `acknowledge_delivery` per peer (`:577–591`), `acknowledge_captures(last_capture())` (`:595–597`), then `legal_intents(player)` (`:603`). `Host::tick` is never called (`docs/adr/0033-godot-bridge.md:124`).

**The T037 surface for v4 sessions** (approved contract, not yet implemented)

- `bind_v4_peer` (`tasks/T037.md:439–441`) — "Binds a protocol v4 session: `bind_v3_peer` with lane-0 version 4 and lane 1 (B-1)"; `HostError::NotV4Peer` is added for a call that needs such a session (`:434`).
- `set_position_withheld` (`tasks/T037.md:443–444`) — "Replaces the entities whose position `peer` is not told (B-8)."
- `position_rows` (`tasks/T037.md:420–421`) — HM-6's per-viewer rows, `PositionView` is stored in `V4State.positions` (`:412`) beside `withheld: Vec<EntityId>` (`:414`).
- `session_wire` (`tasks/T037.md:447–448`) — "returns `4` for a v4 session"; `DeliveryKind`'s doc gains "or v4" (`:448`).
- `DeclareMove` (`tasks/T037.md:1301–1302`) — the turn-based move rides lane 0 and its page carries `Moved { hero, […] }`; the future test K2 asserts `Applied` and the new replica position (`:1305–1306`).
- `take_samples` (`tasks/T037.md:456–462`) — `POLICY_SAMPLE_REFRESH_TICKS: u64 = 20` is "policy, never wire"; `take_samples(peer)` returns "the sample datagrams of the last committed tick not yet taken (C-6)".
- `MoveAdmission` (`tasks/T037.md:468–481`) — four future outcomes of one lane-1 frame: `Admitted { seq }`, `Rejected { seq, code }`, `Duplicate { seq, resent }`, `Refused { seq: Option<u64>, code }`; `ingest_move` is `:483–484`.
- `take_move_receipts` (`tasks/T037.md:485–487`) — "Every released lane-1 receipt not yet taken, in release order; taking them hands them to the reliable stream (D-9)."
- Lane-1 state (`tasks/T037.md:490–496`) — `V4State.lane: MoveLane { next_seq, slots, cache, cache_bytes, bucket, receipts, processed_seq }` and `Host.next_move_order`; the v4 pool and replica on the client side are fed by these bytes.
- What must not change (`tasks/T037.md:1412–1416`) — a wildcard `HostError` mapping and an exhaustive `DeliveryKind` match in `crpg-godot` must keep compiling; T037 adds `NotV4Peer` only (`:433–434`).

## The Godot client

**Scenes and scripts that read the bridge**

- `apps/client/scenes/Main.tscn` (`apps/client/scenes/Main.tscn:1`) — the only scene; `Main.gd` is its root script and it holds the `Proxies` and `Hud` children the scripts sync.
- `scripts/Main.gd` (`apps/client/scripts/Main.gd:1`) — `extends Node3D`; `session = ClassDB.instantiate("CrpgSession")` (`:14`), `session.open(_campaign_dir(), "", "", OPEN_SEED)` (`:19`), one `session.poll()` per `_process` (`:29`), the revision gate `if next.revision() != seen: $Proxies.sync(next); $Hud.sync(next)` (`:40–43`), `choose(i)` → `session.submit_option(seen, i)` (`:45–48`), `session.close()` in `_exit_tree` (`:50–53`), and `CRPG_CAMPAIGN`/`ProjectSettings.globalize_path` for the fixture path (`:55–59`).
- `scripts/SceneSync.gd` (`apps/client/scripts/SceneSync.gd:1`) — `plan(ids)` (`:17`), `sync(view)` (`:39`), `clear()` (`:68`), `proxy_count()` (`:73`); it names each node `proxy_<id>` (`:82`) and sets `proxy.position = Vector3(3.0 * side_index, 0.0, 1.5 * rank)` (`:61`), the label at `y = 1.2` (`:89`) and the mesh lift `mesh_instance.position.y = mesh_height / 2.0` (`:122`). These positions are derived from `net_ids()`/`sides()` only; the view has no position member.
- `scripts/CombatHud.gd` (`apps/client/scripts/CombatHud.gd:1`) — `_ready` only sets `main = get_parent()` (`:60`), `sync(view)` (`:63`) writes round/turn/health/options/log/end, `clear()` (`:85`); it reads exactly the 14 view getters listed in `hud_cases.gd:25–28` and never `placements`/`option_targets` (`apps/client/tests/hud_cases.gd:29–33`).
- `CrpgSession` readers (`apps/client/scripts/Main.gd:14`) — `Main.gd` is the only script that instantiates or calls the session; `SceneSync.gd` and `CombatHud.gd` take a view argument only.

**The test runners and their exit codes**

- `tests/load.gd` (`apps/client/tests/load.gd:2`) — E1 load proof: `quit(2)` when `ClassDB.class_exists("CrpgSession")` is false (`:18–20`), `quit(3)` on the first failed check (`:9`), `quit(0)` on success (`:72`); control `CRPG_CAMPAIGN` missing → open 20 → exit 3 (`:5`).
- `tests/scene_sync.gd` (`apps/client/tests/scene_sync.gd:1`) — runs "the SceneSync cases (S1-S13 and S15-S19)" and checks S14 two frames later (`:2–3`): `quit(3)` when a script fails to load (`:19`, `:24`), `quit(6)` on a case failure (`:29`), `quit(7)` when "no freed proxy is still valid and every kept proxy is" fails (`:3`, `:42`, `:47`), `quit(0)` on success (`:50`).
- `tests/hud.gd` (`apps/client/tests/hud.gd:1`) — runs the HUD cases: `quit(3)` when the case file fails to load (`:13`), `quit(6)` when `run` returns anything but the String `"PASS"` (`:17–19`), `quit(0)` on success (`:22`); its header says "H1-H19" and "H20 (the real session) runs in smoke.gd" (`:2–4`).
- `tests/hud_cases.gd` (`apps/client/tests/hud_cases.gd:1`) — H1–H20 against a `FakeView` "that has exactly the 14 getters the HUD may read" (`:2–3`); `run(root)` returns `"PASS"` or `"H<n>: <detail>"` (`:73`, `:85`); a case returns the token `"ok"` and a crash (`""`) fails (`:4–5`, `:11`); its own H20 is the null-main press (`:6–7`).
- `tests/smoke.gd` (`apps/client/tests/smoke.gd:2`) — E2 headless playthrough: `quit(6)` when a scene-sync case fails (`:28`), `quit(7)` when a HUD case or the end-of-play HUD check fails (`:33`, `:38`, `:75`), `quit(3)` on a failed scene load, open, poll or choose (`:43`, `:48`, `:59`, `:64`, `:89`), `quit(2)` when the class is missing (`:21`), `quit(4)` after 120 s (`:93`), `quit(5)` on a proxy-count mismatch (`:81`), `quit(0)` only on the ended view with `proxy_count() == net_ids().size() == 2` (`:69–78`).
- `tests/scene_sync_cases.gd` (`apps/client/tests/scene_sync_cases.gd:1`) — S1–S18 against an inner `FakeView`; `run(host)` returns `"PASS"` or `"S<n>: detail"` and records the freed/kept nodes S14 checks.

**How they are run**

- Build first (`crates/crpg-godot/AGENTS.md:77`) — `cargo build -p crpg-godot --features engine -j 2`; the `.gdextension` points at `target/<profile>/crpg_godot.dll` (`docs/architecture/crpg-godot.md:137–141`).
- Then, "from Git Bash (not PowerShell)" (`crates/crpg-godot/AGENTS.md:73–85`) — `Godot4 --headless --path apps/client --import`, then `Godot4 --headless --path apps/client -s res://tests/load.gd`, `-s res://tests/scene_sync.gd`, `-s res://tests/hud.gd`, and the control `CRPG_CAMPAIGN=/no/such/dir … -s res://tests/load.gd` (exit 3).
- `Godot4` is a windowed program (`crates/crpg-godot/AGENTS.md:115–117`) — "PowerShell neither waits for it nor sees its exit code: run every `Godot4 ... -s` check from Git Bash."
- `--import` first (`crates/crpg-godot/AGENTS.md:118–119`) — "Run `--import` before any `-s`"; "Its exit code is not a load proof; `load.gd`'s `ClassDB.class_exists` check is."

## Existing tests to copy

**`crates/crpg-godot/src/core/tests.rs`** (`crates/crpg-godot/src/core/tests.rs:1`)

- Scope (`:1`) — "K15-K30 and the open half of K35: the bridge lifecycle, the campaign errors, the revision/ack rules, the NPC driver and the panic fence"; 21 `#[test]` cases.
- Fixture and session helpers (`:24`) — `fixture_root()` (`:24`), `live`/`live_mut` (`:33`, `:40`), `is_fenced` (`:47`), `copy_tree` (`:86`), `open_fixture` (`:100`), `open_and_poll` (`:108`), `open_side` (`:114`), `open_and_poll_side` (`:122`), `id` (`:128`), `ull` (`:132`).
- Authority helpers (`:613`, `:702`) — `mirror_npc` drives an independent `HistoryWorld` oracle for the hash comparison; `crafted(host, player, seq, actor)` builds raw intent bytes with `codec_v2::encode_intent` (`:12`) for the rejection cases.
- Named cases a v4 task can mirror — `fixture_view_is_exact_and_rows_use_the_player_replica_map` (`:139`), `open_failure_codes_and_precedence_leave_no_session` (`:185`), `npc_fires_once_at_the_delay_and_never_twice_in_a_poll` (`:253`), `idle_poll_changes_nothing` (`:328`), `a_full_link_defers_the_next_frame_until_it_is_read` (`:370`), `captures_are_acknowledged_every_poll` (`:481`), `the_authority_hash_matches_an_independent_oracle` (`:515`), `submit_precedence_and_a_successful_send` (`:644`), `corrupt_frame_fences_and_close_resets` (`:825`), `panics_are_caught_at_every_boundary` (`:928`).

**Submodule suites**

- `core/authority/tests.rs` (`crates/crpg-godot/src/core/authority/tests.rs:1`) — "K4-K7, K34 and the direct half of K35": `hand_off`, the NPC chooser and pacer, the pump result fold, the campaign walk and the start-order binding; 7 cases.
- `core/client/tests.rs` (`crates/crpg-godot/src/core/client/tests.rs:1`) — "K8-K14: the client half's rows, events, revisions, receipts, labels and frame refusals, plus the client-owned part of `submit_option`"; 12 cases.
- `core/errors/tests.rs` (`crates/crpg-godot/src/core/errors/tests.rs:1`) — "K1-K2: the closed code table and the host-failure mapping"; 2 cases.
- `core/link/tests.rs` (`crates/crpg-godot/src/core/link/tests.rs:1`) — "K3: the bounded two-ended link": a full direction refuses and changes nothing while FIFO order holds (`:8–10`); 3 cases.

**`crates/crpg-godot/tests/`**

- `tests/bridge.rs` (`crates/crpg-godot/tests/bridge.rs:1`) — K15/BR-1 and K30/B21 "through the public bridge only": `fixture_opens_and_polls_to_the_expected_view` (`:21`) and `close_is_idempotent_through_the_public_api` (`:61`).
- `tests/source_rules.rs` (`crates/crpg-godot/tests/source_rules.rs:1`) — K31/BR-10 "the core stays engine-free and clock-free, the two lower halves never name the authority, and `core` is never a bare path" (`:81–83`), and K32/BR-20 "the shell and root hold exactly one `unsafe`" (`:187–190`); the scanner strips `//` comments and compares whole tokens (`:5–9`).

**The CI `client` job** (`.github/workflows/ci.yml:187`, read only)

- Scope (`:199–223`) — "Classify the change (documentation-only pull requests skip the client gates)"; a documentation-only PR sets `rust=false`, otherwise every gate runs.
- Build (`:225–227`) — `cargo clippy -p crpg-godot --features engine --all-targets --locked -- -D warnings`, then `cargo build -p crpg-godot --features engine --locked`.
- Notice check (`:229–245`) — `apps/client/THIRD_PARTY_NOTICES` must name exactly the eight resolved godot-family packages.
- Godot fetch (`:247–282`) — one exact 4.7.2 asset with SHA-256 and SHA-512 checked before unpacking; a version check against `4.7.2.stable.official.ed1daf0bf`; a seeded `.godot/extension_list.cfg`.
- Gates (`:284–332`) — `--import` (`:284`), E1 `load.gd` (`:293`) and its missing-campaign control expecting 3 (`:302`), BR-22 `smoke.gd` (`:313`) and its control expecting 3 (`:322`).
- Artifact (`:333–354`) — stages `target/debug/crpg_godot.dll`, the tracked `apps/client` files, the fixture campaign and both licences as `crpg-client-slice-windows-x86_64`; `scene_sync.gd` and `hud.gd` are local gates only, not CI steps.

## Rules that bind it

(section pending)

## Agent log

- 2026-11-15 (UTC) · deepseek-v4.1-flash + T151 facts · Skeleton with the
  required section headings, so each section lands as its own reviewed commit.
