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

## Host start-up today

## Existing binaries and their layout

## CI today

## Rules that bind it

## Agent log
