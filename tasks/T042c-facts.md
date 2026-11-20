# T042c facts: hostile-server follow-up (`crpg-net-quic` and the Godot bridge)

Fact sheet for `T042c`. Facts only: no recommendations, no proposed types, no opinions.
Each entry cites an existing `path:line`.

## Requirements

- `crpg-net hostile-input mandate` (`docs/CRPG_ENGINE_SPEC.md:1488`): malicious-client tests are a first-class suite; every case must give a clean rejection, never a panic, and the desync test runs 5,000 ticks. This sentence names the server side only.
- `crpg-net deliverable` (`docs/CRPG_ENGINE_SPEC.md:1586`): the `crpg-net` deliverable includes the simulated-network transport and the malicious-client suite.
- `crpg-net tests` (`docs/CRPG_ENGINE_SPEC.md:1587`): the test list includes every malicious-client case and reconnection.
- `top-risk 21` (`docs/CRPG_ENGINE_SPEC.md:1703`): the malicious-client test suite is item 21 of the risk mitigation list.
- `one authoritative server` (`docs/CRPG_ENGINE_SPEC.md:969`): one platform-neutral authoritative server runs in-process on Windows single-player; it "cannot mutate authoritative state directly" (`docs/CRPG_ENGINE_SPEC.md:971`).
- `server-side refusal has no spec client-side twin` (`docs/CRPG_ENGINE_SPEC.md:1488`): no other binding sentence in the spec requires a client to refuse hostile server bytes; "none found" for `Welcome`, bridge faults and the client close.
- `ADR-0024 placement` (`docs/adr/0024-separate-crpg-net-quic-crate.md:58`): the QUIC crate owns ALPN, framing, pinning and close codes; the wire record is ADR-0025.
- `ADR-0025 no negotiation` (`docs/adr/0025-lane0-quic-mapping-and-pinned-trust.md:35`): the server accepts the one chosen wire version or refuses it; nothing is negotiated.
- `ADR-0025 welcome layout` (`docs/adr/0025-lane0-quic-mapping-and-pinned-trust.md:43`): the welcome is exactly 17 bytes, and its version must equal the hello's (`docs/adr/0025-lane0-quic-mapping-and-pinned-trust.md:50`).
- `ADR-0025 pinned trust` (`docs/adr/0025-lane0-quic-mapping-and-pinned-trust.md:56`): a certificate chain is refused; the pin is the only trust input.
- `ADR-0025 close codes` (`docs/adr/0025-lane0-quic-mapping-and-pinned-trust.md:70`): application close codes 0–9, each sent with its name; `VersionRefused` is 6 (`docs/adr/0025-lane0-quic-mapping-and-pinned-trust.md:81`).
- `ADR-0026 auth before version` (`docs/adr/0026-quic-host-adapter.md:46`): a bad credential and a wrong-length credential are `AuthRefused` (5), checked before `VersionRefused` (6) — server side.
- `ADR-0026 fencing closes` (`docs/adr/0026-quic-host-adapter.md:63`): unbind, revocation and narrowing grants close the connection with `SessionFenced` (9) in `Discard` mode — server side.
- `ADR-0033 fatal table` (`docs/adr/0033-godot-bridge.md:341`): 52 `FRAME_UNDECODABLE` is a `CodecError` on a delivered frame, fatal, "stop; show text"; 53 `REPLICA_REJECTED` (`docs/adr/0033-godot-bridge.md:342`) is T046 refusing a frame; 54 `UNEXPECTED_TRANSFER` (`docs/adr/0033-godot-bridge.md:343`) is a `DeliveryKind::Transfer` in the slice.
- `ADR-0033 BR-8` (`docs/adr/0033-godot-bridge.md:641`): a `Transfer` entry routes to fatal 54.
- `ADR-0033 BR-13` (`docs/adr/0033-godot-bridge.md:646`): a corrupt frame on the link gives fatal 52, the host is closed, and every later `poll` repeats it.
- `ADR-0035 fault split` (`docs/adr/0035-client-replica.md:13`): the replica applies whole decoded values, so codec faults stay code 52 and replica faults code 53.
- `T042 out of scope` (`tasks/T042.md:653`): T042a excludes "`crpg-net-quic` framing (ADR-0025's length prefix and close codes)".
- `T042b out of scope` (`tasks/T042b.md:1375`): the QUIC `Welcome` decode and client close (`crates/crpg-net-quic`), and the Godot bridge's fault mapping and replica refusals (`crates/crpg-godot`), are the follow-up filed as T042c (`tasks/T042b.md:1377`).
- `T042b case tables` (`tasks/T042b.md:391`): the delta op-tag table; (`tasks/T042b.md:410`) the header/op-count rows; (`tasks/T042b.md:438`) the Health/Receipt rows; (`tasks/T042b.md:593`) the ActionResolved rows; (`tasks/T042b.md:715`) the Moved/Position rows.
- `T042b mutant tables` (`tasks/T042b.md:1300`): T042b1's MA1–MA12; (`tasks/T042b.md:1315`) MB1–MB10; (`tasks/T042b.md:1330`) MC1–MC10; (`tasks/T042b.md:1345`) MD1–MD10.
- `D65b` (`tasks/DECISIONS-D65.md:11`): "open my campaign to others" is built later, after the demo; the run loop must let one `Host` serve an in-memory local player and QUIC players at once (`tasks/DECISIONS-D65.md:15`).

## T042b today

## crpg-net-quic client

## Godot bridge

## Test harness options

## Rules that bind it

## Agent log

- 2026-11-20 (UTC) · opencode-go/deepseek-v4.1-flash + T042c facts · Gathering the requirements, existing suites, refusal paths and constraints the T042c contract will be written from.
