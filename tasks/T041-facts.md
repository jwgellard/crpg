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

TODO

## Hosts and run loops today

TODO

## Filtering and visible state

TODO

## Existing testkit and test patterns

TODO

## Dedicated server today

TODO

## Rules that bind it

TODO

## Agent log

- 2026-11-19 (UTC) · opencode-go/muse-spark-1.3-contributor + T041 facts · Skeleton with all section headings so later edits fill one section per commit.
