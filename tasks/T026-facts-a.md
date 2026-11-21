# T026 facts A — Godot movement (protocol, host and rules side)

## Requirements

- `Presentation + input + prediction of own movement only.` (`docs/CRPG_ENGINE_SPEC.md:24`) — the client layer owns prediction of own movement only.
- `The client renders at display refresh and interpolates between the last two received snapshots with a fixed ~100 ms delay buffer.` (`docs/CRPG_ENGINE_SPEC.md:321`)
- `Server pathfinding uses **Detour** or a Rust equivalent over the same navmesh. Server and client must use the identical navmesh file, or clients will predict movement the server rejects.` (`docs/CRPG_ENGINE_SPEC.md:731`)
- `MoveTo { position, tick },` (`docs/CRPG_ENGINE_SPEC.md:799`) — illustrative move intent shape.
- `### 7.5 Prediction, reconciliation, interpolation` (`docs/CRPG_ENGINE_SPEC.md:827`) — section heading.
- `Client predicts own movement: applies input immediately, keeps a ring buffer of (tick, input, predicted_position).` (`docs/CRPG_ENGINE_SPEC.md:829`)
- `All other entities are interpolated between the last two snapshots with a fixed ~100 ms buffer.` (`docs/CRPG_ENGINE_SPEC.md:831`)
- `**Nothing else is predicted.** An attack shows a wind-up animation and a "pending" state until the server resolves it.` (`docs/CRPG_ENGINE_SPEC.md:832`)
- `*Note 2026-09-30 (E017, D13/D24, T033):* movement prediction waits for an authoritative movement/nav specification ...` (`docs/CRPG_ENGINE_SPEC.md:833`)
- `Input            action map, click-to-move, hotbar, controller` (`docs/CRPG_ENGINE_SPEC.md:876`)
- `No authoritative logic. The client may compute *predictions* and *display hints* ...` (`docs/CRPG_ENGINE_SPEC.md:887`)
- `Click-to-move over a baked navmesh, with server authority and client prediction.` (`docs/CRPG_ENGINE_SPEC.md:1522`)
- `Deliverables: ... crpg-godot bridge, scene sync, camera, click-to-move with prediction, basic HUD, combat log, dialogue UI, ...` (`docs/CRPG_ENGINE_SPEC.md:1598`)
- `*Work:* a Rust GDExtension holding an array of 200 ... interpolates between updates ...` (`docs/CRPG_ENGINE_SPEC.md:1913`)
- `*Work:* two processes; client sends MoveTo intents, server integrates at 20 Hz and returns positions with an input acknowledgement; client predicts and reconciles. Add a latency/jitter/loss shim.` (`docs/CRPG_ENGINE_SPEC.md:1921`)
- `*Test:* smooth local movement at 150 ms RTT and 3% loss; reconciliation does not visibly rubber-band; ...` (`docs/CRPG_ENGINE_SPEC.md:1922`)
- `transport for authoritative movement with client-side prediction of the local player only (§7.5)` (`docs/adr/0004-quic-movement-spike.md:9`)
- `smooth local movement at 150 ms RTT / 3% loss with non-visible reconciliation` (`docs/adr/0004-quic-movement-spike.md:11`)
- `MoveTo { seq, target }-shaped client intents as QUIC datagrams` (`docs/adr/0004-quic-movement-spike.md:19`)
- `predicts locally with the *same* deterministic step function the server uses, buffers (seq, target, predicted_pos)` (`docs/adr/0004-quic-movement-spike.md:23`)
- `standard client prediction/reconciliation (§7.5)` (`docs/adr/0004-quic-movement-spike.md:27`)
- `Divergence is the distance between what the client had predicted for an acked seq and what the server actually says` (`docs/adr/0004-quic-movement-spike.md:48`)
- `"Visible" (>0.5 world-units ...)` (`docs/adr/0004-quic-movement-spike.md:50`)
- `Conditional go. The reconciliation algorithm is validated` (`docs/adr/0004-quic-movement-spike.md:89`)
- `Proceed with QUIC / quinn and the predict-and-reconcile design as specified.` (`docs/adr/0004-quic-movement-spike.md:92`)
- `The prediction/reconciliation implementation in this spike (src/bin/client.rs) is a reasonable reference ... ring buffer of buffered inputs, snap-and-replay on ack` (`docs/adr/0004-quic-movement-spike.md:162`)
- `T036 (authoritative movement, S-set), T026p (lane-1 movement, S-set), T037` (`docs/adr/0036-movement-and-navigation.md:14`)
- `processed-seq/server-tick acks; lane 0 stays window 1; prediction is per-tick MoveTo ...` (`docs/adr/0036-movement-and-navigation.md:51`)
- `per-tick MoveTo { seq, target } datagrams with latest-seq-wins.` (`docs/adr/0036-movement-and-navigation.md:53`)
- `The client's prediction and previews call them (crpg-godot may ...)` (`docs/adr/0036-movement-and-navigation.md:85`)
- `and the client preview (G3) pass the clicked point through snap` (`docs/adr/0036-movement-and-navigation.md:328`)
- `G1. T049, interpolation. Each entity has a sample buffer keyed by server tick. Render at the newest tick minus the U7 delay, interpolating linearly ...` (`docs/adr/0036-movement-and-navigation.md:446`)
- `G2. T026, prediction. Only for actors the peer controls, and never in TB ...` (`docs/adr/0036-movement-and-navigation.md:453`)
- `G3. T050, click-to-move. Pick a ground point and pass it through the same snap as the host (S3); preview the path and TB cost ... send MoveTo or DeclareMove by mode; never send a path.` (`docs/adr/0036-movement-and-navigation.md:460`)
- `The crpg-godot to crpg-nav path edge needs approval.` (`docs/adr/0036-movement-and-navigation.md:463`)
- `v1, v2 and T024 frozen.` (`docs/adr/0036-movement-and-navigation.md:497`)
- `none found` for `docs/adr/0035-client-replica.md` (grep terms absent).
- `Sole crate: crpg-godot, Phase 5. D13 scope selected` (`tasks/T026.md:5`)
- `E015 requires speculative state in a separate local movement buffer, outside sim and the immutable authoritative replica. Reconcile at the acknowledged seq, then replay newer local inputs using the approved movement rules.` (`tasks/T026.md:29`)
- `Server tick is time, seq is identity; neither grants client scheduling authority. Never predict attacks, damage, resources, turns or remote entity authority.` (`tasks/T026.md:32`)
- `Remote entities interpolate only permitted samples. Old/out-of-order ack must not regress accepted state; explicit resync/fenced epoch clears incompatible buffered inputs.` (`tasks/T026.md:33`)
- `No hidden entities can appear through interpolation history after revocation.` (`tasks/T026.md:38`)
- `Spatial presentation smoothing cannot alter authoritative hashes or receipts.` (`tasks/T026.md:39`)
- `ADR-0004's 150 ms RTT / ±15 ms jitter / 3% loss scenario is required` (`tasks/T026.md:54`)
- `Add protocol version 4 to crpg-net: movement on the wire, as ADR-0036 L1–L6 decided` (`tasks/T026p.md:44`)
- `A v4 session uses two lanes: lane 0 (version byte 4) keeps every v3 message and window 1, and gains the turn-based move verb and two movement ops; lane 1 is new, is its own separately versioned subprotocol (first byte 0xA1, D13)` (`tasks/T026p.md:48`)
- `Samples are self-contained datagrams of at most 1,024 bytes and 38 entries.` (`tasks/T026p.md:209`)
- `Disclosure: a reliable anchor, then unreliable samples.` (`tasks/T026p.md:220`)
- `Positions after a reconnect come in the first page after the snapshot, not in the 0x82 body.` (`tasks/T026p.md:238`)
- `Moved is a lane-0 event at the history slot.` (`tasks/T026p.md:251`)
- `The client's lane-1 sender state lives in crpg-net. MoveOutbox holds the lane-1 rules from the client side (sequence, window 8, one receipt per intent in order, resend after a resume)` (`tasks/T026p.md:287`)
- `The interpolation delay is client policy (T049).` (`tasks/T026p.md:300`)
- `## D13 — movement/prediction: ordered scope, unchanged dependency graph` (`tasks/POST-T018-DECISIONS.md:310`)
- `Movement gets a separately versioned lane and processed-seq/server-tick acks; lane-0 remains window 1. Local-player movement prediction is outside immutable replica/sim, no combat prediction. Other actors interpolate permitted state. Do not raise the lane-0 window in this milestone.` (`tasks/POST-T018-DECISIONS.md:318`)

## Lane-1 wire today

- `movement_v4::MOVEMENT_VERSION` (`crates/crpg-net/src/movement_v4.rs:32`) = `0xA1`, first byte of every lane-1 message.
- `movement_v4::LANE_MOVEMENT` (`crates/crpg-net/src/movement_v4.rs:34`) = 1.
- `movement_v4::MAX_MOVE_DOWN_BYTES` (`crates/crpg-net/src/movement_v4.rs:50`) = 1,024, largest lane-1 downstream message.
- `movement_v4::MAX_SAMPLE_ENTRIES` (`crates/crpg-net/src/movement_v4.rs:57`) = 38.
- `movement_v4::POLICY_MOVE_INTENTS_PER_SEC` / `_BURST` / `_IN_FLIGHT` (`crates/crpg-net/src/movement_v4.rs:67`) = 20 / 40 / 8 (policy, never wire).
- `movement_v4::MoveBody` (`crates/crpg-net/src/movement_v4.rs:76`) — `MoveTo { destination: Point }` or `Halt`.
- `movement_v4::MoveIntent` (`crates/crpg-net/src/movement_v4.rs:89`) — `epoch`, `seq`, `actor`, `body`; lane implied.
- `movement_v4::MoveOutcome` (`crates/crpg-net/src/movement_v4.rs:102`) — `Accepted { start_tick }`, `Superseded`, `Rejected(RejectionCode)`.
- `movement_v4::MoveReceipt` (`crates/crpg-net/src/movement_v4.rs:116`) — `epoch`, `seq`, `actor`, `processed_tick`, `outcome`.
- `movement_v4::SampleEntry` (`crates/crpg-net/src/movement_v4.rs:131`) — `entity`, `point`, `moving`.
- `movement_v4::MoveSample` (`crates/crpg-net/src/movement_v4.rs:143`) — `server_tick`, `processed_seq`, `entries`.
- `movement_v4::MoveDown` (`crates/crpg-net/src/movement_v4.rs:154`) — `Receipt(MoveReceipt)` or `Sample(MoveSample)`.
- `movement_v4::encode_intent` (`crates/crpg-net/src/movement_v4.rs:207`) and `decode_intent` (`crates/crpg-net/src/movement_v4.rs:228`) — lane-1 intent codec.
- `movement_v4::encode_down` (`crates/crpg-net/src/movement_v4.rs:273`) and `decode_down` (`crates/crpg-net/src/movement_v4.rs:318`) — lane-1 downstream codec.
- `movement_v4::check_sample` (`crates/crpg-net/src/movement_v4.rs:432`) and `sample_datagrams` (`crates/crpg-net/src/movement_v4.rs:447`) — sample validation and chunking.
- `protocol_v4::PROTOCOL_VERSION` (`crates/crpg-net/src/protocol_v4.rs:27`) = 4.
- `protocol_v4::INTENT_TAG_DECLARE_MOVE` (`crates/crpg-net/src/protocol_v4.rs:30`) = 6 (lane-0 turn-based move).
- `protocol_v4::DELTA_TAG_MOVED` (`crates/crpg-net/src/protocol_v4.rs:32`) = 21 and `DELTA_TAG_POSITION` (`crates/crpg-net/src/protocol_v4.rs:34`) = 22.
- `protocol_v4::COORD_LIMIT_RAW` (`crates/crpg-net/src/protocol_v4.rs:38`) = 2^29 and `MAX_MOVED_POINTS` (`crates/crpg-net/src/protocol_v4.rs:41`) = 64.
- `protocol_v4::Point` (`crates/crpg-net/src/protocol_v4.rs:45`) — bounded `[i32; 3]`.
- `protocol_v4::PositionFact` (`crates/crpg-net/src/protocol_v4.rs:67`) — disclosed position plus moving flag.
- `protocol_v4::DeltaOp::Moved` (`crates/crpg-net/src/protocol_v4.rs:160`) and `DeltaOp::Position` (`crates/crpg-net/src/protocol_v4.rs:168`).
- `codec_v4::encode_intent` / `decode_intent` (`crates/crpg-net/src/codec_v4.rs:181` / `:223`) and `encode_delta` / `decode_delta` (`crates/crpg-net/src/codec_v4.rs:402` / `:428`).
- `codec_v4::frame_spans` (`crates/crpg-net/src/codec_v4.rs:375`) — v4 page splitting.
- `positions_v4::PositionRow` (`crates/crpg-net/src/positions_v4.rs:36`), `PositionView` (`crates/crpg-net/src/positions_v4.rs:54`), `project_page_v4` (`crates/crpg-net/src/positions_v4.rs:116`), `project_samples` (`crates/crpg-net/src/positions_v4.rs:199`), `project_snapshot_v4` (`crates/crpg-net/src/positions_v4.rs:252`).
- `projection_v4::EventCandidate::Moved` (`crates/crpg-net/src/projection_v4.rs:18`) and `project_events` (`crates/crpg-net/src/projection_v4.rs:41`).
- `pool_v4::ReassemblyPoolV4` (`crates/crpg-net/src/pool_v4.rs:102`), reusing `PublicationV3` and `ReceiverStatusV3` (`crates/crpg-net/src/pool_v4.rs:28`).
- `replica_v4::ReplicaWorldV4` (`crates/crpg-net/src/replica_v4.rs:55`), `PositionState` (`crates/crpg-net/src/replica_v4.rs:32`), `SampleClock` (`crates/crpg-net/src/replica_v4.rs:43`), `apply_delta` / `apply_snapshot` / `apply_sample` (methods of `ReplicaWorldV4`).
- `outbox_v4::MoveOutbox` (`crates/crpg-net/src/outbox_v4.rs:54`) and `OutboxError` (`crates/crpg-net/src/outbox_v4.rs:17`).
- `crpg-net` lane-0 `IntentFrame` carries `observed_tick` (`crates/crpg-net/src/protocol_v4.rs:78`); lane-1 `MoveIntent` has no `observed_tick`.

## Movement host

- `QuicServer::bind_movement` (`crates/crpg-net-quic/src/server.rs:248`) and `bind_movement_resumable` (`crates/crpg-net-quic/src/server.rs:255`) — movement endpoints.
- `crpg-server::main.rs` uses `QuicServer::bind_movement_resumable` (`crates/crpg-server/src/main.rs:753`) when `movement` is true.
- `Host::bind_v4_peer` (`crates/crpg-server/src/host.rs:1377`) — binds a v4 (movement-capable) session.
- `Host::set_position_withheld` (`crates/crpg-server/src/host.rs:1433`) — publishes position withdrawal/anchor to one session.
- `Host::bind_nav_mesh` (`crates/crpg-server/src/host.rs:4580`), `bind_area_bounds` (`crates/crpg-server/src/host.rs:4618`), `nav_revision` (`crates/crpg-server/src/host.rs:4632`), `place` (`crates/crpg-server/src/host.rs:4643`), `move_to` (`crates/crpg-server/src/host.rs:4663`), `halt` (`crates/crpg-server/src/host.rs:4684`), `host_move` (`crates/crpg-server/src/host.rs:4708`) — movement host operations.
- `Host::take_samples` (`crates/crpg-server/src/host.rs:3676`) — returns lane-1 sample datagrams; `POLICY_SAMPLE_REFRESH_TICKS` (`crates/crpg-server/src/host.rs:131`) = 20.
- `Host::ingest_move` (`crates/crpg-server/src/host.rs:3715`) and `take_move_receipts` (`crates/crpg-server/src/host.rs:3893`) — lane-1 admission and receipts.
- `QuicHost::register_v4_invitation` (`crates/crpg-server/src/quic.rs:2530`) and `advance` (`crates/crpg-server/src/quic.rs:2721`) — `/3` route.
- `QuicHost::start_resumable` path binds a nav mesh (`crates/crpg-server/src/quic.rs:3848`) — dedicated server binds M10 mesh.
- Embedded single-player host built in `crates/crpg-godot/src/core/mod.rs:440` via `Host::from_history` with `ProtocolSelection::V2` (`crates/crpg-server/src/host.rs:986`), `bind_area_bounds` (`crates/crpg-godot/src/core/mod.rs:451`), `bind_v4_peer` (`crates/crpg-godot/src/core/mod.rs:454`), `bind_peer` for NPC (`crates/crpg-godot/src/core/mod.rs:466`); no `bind_nav_mesh` call in `crates/crpg-godot` (grep: none found). `LocalHost::new` (`crates/crpg-server/src/run.rs:348`) wraps it.
- `HostConfig` / `ProtocolSelection` (`crates/crpg-server/src/host.rs:147` / `:175`) — only V1/V2 in crpg-server; v4 peer binding runs on a V2 host.
- 256-entry answer cache referenced in `crates/crpg-server/src/host_v4.rs:154` (lane ledger retirement path).

## Movement rules and nav

## Tests to copy

## Open follow-ups

## Agent log

- 2026-11-21 (UTC) · opencode-go/beam-free + T026 facts A · Fact sheet A gathering protocol, host and rules-side movement facts for Godot client movement.