# T041 facts — scripted headless clients and desync suite (fact sheet)

## Requirements

- `Desync test: 5,000 ticks` (`docs/CRPG_ENGINE_SPEC.md:1488`) — run server and client replicas over the simulated network for 5,000 ticks and assert the replica's visible state matches the server's filtered projection.
- `Malicious-client suite with log line` (`docs/CRPG_ENGINE_SPEC.md:1488`) — oversized packets, malformed frames, unowned/illegal-time intents, flooding, packet replay, out-of-interest requests; each gets a clean rejection and a log line, never a panic.
- `Phase 4 tests` (`docs/CRPG_ENGINE_SPEC.md:1587`) — desync test over 5,000 ticks with loss and jitter; every malicious-client case; reconnection.
- `Phase 4 definition of done` (`docs/CRPG_ENGINE_SPEC.md:1588`) — two headless scripted clients connect, move, fight, and end with identical visible state.
- `Phase 4 platform acceptance` (`docs/CRPG_ENGINE_SPEC.md:1589`) — one reusable authoritative host serves Windows embedded single-player, Windows dedicated, and Linux dedicated adapters, with real smoke tests as each exists.
- `Regression/detection facility` (`docs/CRPG_ENGINE_SPEC.md:1462`) — the replay/golden facility exists to give regression detection, bisectable changes, save/load verification, and multiplayer desync detection.
- `Deterministic pairs` (`docs/CRPG_ENGINE_SPEC.md:685`) — `pairs` replaced with a deterministic ordered iterator, preventing a class of desync and replay bugs.
- `Launch N clients` (`docs/CRPG_ENGINE_SPEC.md:1074`) — editor "Launch N clients" spawns real client processes against its server with a latency/jitter/loss simulator in front.
- `MVP two-client check` (`docs/CRPG_ENGINE_SPEC.md:1527`) — run the same campaign against a separate `crpg-server` process with two clients connected and verify both see the same world.
- `T2 QUIC movement spike shim` (`docs/CRPG_ENGINE_SPEC.md:1921`) — two processes with `MoveTo` intents at 20 Hz plus a latency/jitter/loss shim.
- `T18 in-memory transport` (`docs/CRPG_ENGINE_SPEC.md:2081`) — in-memory transport with configurable latency, jitter, loss, and reorder, plus `assert_transport` conformance tests.
- `T18 two-World desync test` (`docs/CRPG_ENGINE_SPEC.md:2082`) — two-`World` desync test over 5,000 ticks with 3% loss.
- `Estimation Medium items` (`docs/CRPG_ENGINE_SPEC.md:1681`) — §19.2 lists interest sets plus per-client filtering with a leak test (#10), the malicious-client suite (#21), and the multi-client test launcher with network simulation (#24).
- `Desync as semantic comparison` (`docs/adr/0009-determinism-scope.md:46`) — the cross-machine check is the desync test: replica visible state matches the server's filtered projection, a semantic comparison, never hash equality.
- `H-filter decision` (`docs/adr/0034-interest-projection.md:9`) — `crpg-net` needed the H-filter decision that decides what each player's client may receive.
- `Output is ops, not frames` (`docs/adr/0034-interest-projection.md:18`) — new `interest` module turns viewer context into ops; frames, chunking, and sequence numbers stay with the host.
- `Client replica need` (`docs/adr/0035-client-replica.md:9`) — the Godot bridge needs an engine-neutral client copy of one filtered v2 session, holding what the wire disclosed in order.
- `M4 exit criteria` (`docs/IMPLEMENTATION_PLAN.md:171`) — two headless scripted clients over real QUIC with identical visible state (#1), 5,000-tick loss/jitter desync (#2), malicious suite (#3), 30-second-grace reconnection (#4), gate 10 save/load (#5), gate 12 artifacts (#6), gate 13 smoke on all three hosts (#7).
- `T041 row` (`docs/IMPLEMENTATION_PLAN.md:246`) — `crpg-testkit`, after T025b/T027c/T037/T039: scripted headless clients, 5,000-tick loss/jitter desync, two-client identical visible state, gate 13 dedicated smoke.
- `T042 row` (`docs/IMPLEMENTATION_PLAN.md:247`) — wire-level malicious-client suite (spec §19.2 #21); host-level cases live in T041.
- `T070 acceptance row` (`docs/IMPLEMENTATION_PLAN.md:335`) — automated §17.2 acceptance after T069 and T041, smoking all three hosts with two clients.
- `Host/wire split` (`tasks/T042.md:39`) — T042 is the wire level in `crpg-net`; host-level cases live in T041.
- `D67a log-line decision` (`tasks/DECISIONS-D67.md:9`) — T042 pins each rejection and its exact text now; actually writing the line is the server program's job (T040 or T041), filed as a backlog row with a "where do server logs go" decision there.
- `Server log-line backlog row` (`tasks/BACKLOG.md:341`) — proposed: server log line per rejected client message plus the log-destination decision; belongs to T040 or T041 (D67a).
- `Gate-13 row` (`tasks/BACKLOG.md:506`) — integrated smoke: post-T018 real capabilities, Windows embedded/dedicated plus Linux dedicated smoke tests, no placeholder jobs.
- `Gate-10 campaign breadth` (`tasks/BACKLOG.md:503`) — per-target in-process save/load equivalence on the T022 fixture authority now; campaign-wide breadth comes with T040/T041, the first tasks building a host from campaign data.

## Clients today

- `ClientSession`: none found — no type by that name exists in `crpg-net`, `crpg-net-quic`, or `crpg-server`.
- `Transport` (`crates/crpg-net/src/transport.rs:14`) — `pub trait Transport` with `send(&[u8])` and `recv() -> Option<Vec<u8>>`; the byte pipe between two in-process endpoints, no threads/clock/I/O.
- `InMemoryTransport` (`crates/crpg-net/src/sim.rs:396`) — the T018b simulated fabric implementing the byte pipe with bounded queues, injected time, and per-peer fault schedules.
- `FaultSchedule` (`crates/crpg-net/src/sim.rs:321`) — deterministic loss/dup/reorder knobs: `loss_every` drops every nth datagram per peer (`crates/crpg-net/src/sim.rs:323`), `dup_every` duplicates (`crates/crpg-net/src/sim.rs:325`), `reorder_depth` bounds displacement (`crates/crpg-net/src/sim.rs:327`).
- `InMemoryTransport::tick` (`crates/crpg-net/src/sim.rs:572`) — deterministic pump moving staged datagrams through faults into ready queues in ascending `PeerId` order.
- `Protocol versions` (`crates/crpg-net/src/protocol.rs:25`) — v1 `PROTOCOL_VERSION: u8 = 1`, v2 (`crates/crpg-net/src/protocol_v2.rs:34`), v3 (`crates/crpg-net/src/protocol_v3.rs:40`), v4 (`crates/crpg-net/src/protocol_v4.rs:27`); selection is explicit, no autodetection.
- `SessionFence` (`crates/crpg-net/src/resume.rs:74`) — orders connection generations and checks epoch/fence order; the host's explicit `ResumeDecision` is the only input producing `Accepted`.
- `ReplicaWorld` (`crates/crpg-net/src/replica.rs:121`) — engine-neutral client copy of one filtered v2 session, applied via `apply_delta` (`crates/crpg-net/src/replica.rs:188`); the conformance `Replica` is a test oracle, not a shipped type.
- `ReplicaWorldV4` (`crates/crpg-net/src/replica_v4.rs:55`) — v4 client replica with positions; `apply_sample` never fails (`crates/crpg-net/src/replica_v4.rs:148`) since lane-1 datagrams are unordered and lossy.
- `MoveOutbox` (`crates/crpg-net/src/outbox_v4.rs:54`) — client-side lane-1 intent sender; `send` queues a `MoveIntent` (`crates/crpg-net/src/outbox_v4.rs:93`), `receive` takes only the oldest pending intent.
- `QuicClient` (`crates/crpg-net-quic/src/client.rs:104`) — one connection, one UDP socket, one I/O thread with a synchronous non-blocking API over bounded queues (`crates/crpg-net-quic/src/client.rs:1`); `connect` dials a server (`crates/crpg-net-quic/src/client.rs:118`).
- `Client module set` (`crates/crpg-net/src/lib.rs:66`) — `replica`, `replica_v3`, `replica_v4`, `outbox_v4`, `movement_v4`, `interest`, `sim`, and `transport` are all net-local modules a headless client assembles without Godot.

## Hosts and run loops today

- `Host` (`crates/crpg-server/src/host.rs:935`) — the in-memory authoritative host; `Host::new(seed, config, now_ms)` builds a fresh authority (`crates/crpg-server/src/host.rs:973`), `Host::from_history` wraps an embedding-choreographed one with an empty journal (`crates/crpg-server/src/host.rs:980`).
- `HostLoop` (`crates/crpg-server/src/run.rs:110`) — one cycle order (intake, AI stage, advance, deliver, retire captures) on a 20 Hz grid driven by injected `now_ms` (`crates/crpg-server/src/run.rs:4`); `Hosting` supplies the five stages (`crates/crpg-server/src/run.rs:36`), `drive_ai` runs the AI hook if any (`crates/crpg-server/src/run.rs:45`).
- `LocalHost` (`crates/crpg-server/src/run.rs:332`) — one `Host` plus in-memory local links; the AI stage's hook is the embedding's `AiHook` (`crates/crpg-server/src/run.rs:237`), settable via `set_ai` (`crates/crpg-server/src/run.rs:396`).
- `LocalClient` (`crates/crpg-server/src/run.rs:818`) — the peer end of a local link, implementing net's `Transport` (`crates/crpg-server/src/run.rs:834`); today the closest thing to a headless client harness.
- `RunnerHandle` (`crates/crpg-server/src/runner.rs:4`) — `spawn` drives a `HostLoop` on a background thread at its cadence; tests use an injected, manually stepped clock, never wall time.
- `QuicHost` (`crates/crpg-server/src/quic.rs:528`) — the QUIC adapter over `crpg-net-quic`; `pump(now_ms)` drives handshakes, intake, and delivery (`crates/crpg-server/src/quic.rs:631`).
- `Host::drive_ai` (`tasks/T153.md:60`) — one trusted host operation the embedding calls between ticks, picking AI-controlled combatants and applying each `Decision` through existing host operations.
- `save_host_file` (`crates/crpg-server/src/save.rs:272`) — the T039 checkpoint/save adapter: saves a quiescent host through `crpg-persist::save_file`; `load_checkpoint` restores one (`crates/crpg-server/src/checkpoint.rs:176`).

## Filtering and visible state

- `Tier-1 interest projection` (`crates/crpg-net/src/interest.rs:1`) — one viewer, one area, host-supplied facts; `InterestView` turns host-authorized rows into v1 `DeltaOp` values or one T024 `SnapshotBody`, adding no wire form.
- `ViewerContext` (`crates/crpg-net/src/interest.rs:74`) — one coherent per-call context: session epoch, authoritative tick, one-area interest set, entity facts, despawned keys, turn state plus grants.
- `project_delta` (`crates/crpg-net/src/interest.rs:225`) — projects one delta page for the viewer's context: visible rows, entering keys minted, differences only.
- `project_snapshot` (`crates/crpg-net/src/interest.rs:334`) — snapshot variant: keeps live visible ids, retires removed keys with no op, records the body so the next delta emits differences; always advances the cursor.
- `Snapshot transfer promotion` (`tasks/T024.md:8`) — T018b's provisional test-local snapshot tracker promoted into a production transfer wire and bounded reassembler.
- `Position grant` (`tasks/T037.md:51`) — position anchors and samples sent under the per-viewer position grant; `HistoryEvent::Moved` published alongside.
- `Authority-hash oracle` (`crates/crpg-testkit/tests/host_movement.rs:375`) — the host suite pins `host.authority_hash() == history_hash(oracle)`; equality is over the authority, not over a client view.
- `Hash sequences over World` (`crates/crpg-testkit/src/harness.rs:186`) — `run_hash_sequence` records `state_hash(&world)` per tick; no existing equality or hash check over a client's replica view was found.

## Existing testkit and test patterns

TODO

## Dedicated server today

TODO

## Rules that bind it

TODO

## Agent log

- 2026-11-19 (UTC) · opencode-go/muse-spark-1.3-contributor + T041 facts · Skeleton with all section headings so later edits fill one section per commit.
