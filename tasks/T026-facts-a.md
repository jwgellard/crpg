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

## Movement host

## Movement rules and nav

## Tests to copy

## Open follow-ups

## Agent log

- 2026-11-21 (UTC) · opencode-go/beam-free + T026 facts A · Fact sheet A gathering protocol, host and rules-side movement facts for Godot client movement.