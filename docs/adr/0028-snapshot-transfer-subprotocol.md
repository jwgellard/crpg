# ADR-0028: Snapshot transfer subprotocol

Date: 2026-10-25 (UTC)
Status: **Accepted** — filed by T024.

## Context

`crpg-net` carries a separate versioned snapshot-transfer subprotocol beside
frozen v1/v2 combat protocol. First byte `0x81`, lane 2, downstream tags
0x10–0x12, upstream 0x20–0x21. See `tasks/T024.md` S§2 and S§4.

## Decision

1. Separate subprotocol, not a main protocol version (first byte 0x81).
2. Tags disjoint from combat tags.
3. Packing: 1..=8 segments of 4,096 bytes per chunk.
4. Two layers: byte layer `TransferAssembly` proves full 1 MiB / 32-chunk envelope; receiver layer `ReassemblyPool` adds deadline, budgets, body validation, atomic publication, cutover gate.
5. Cutover by delivery cursor.
6. Fail closed.

Sender rules and host requirements in T024 S§11.

## Authority and affected surface

- Owning crate: `crates/crpg-net` modules `protocol_transfer`, `codec_transfer`, `reassembly`.
- Contract: `tasks/T024.md` S§0–S§16.

## Compatibility, hash and dependency impact

- No new dependency. v1/v2 untouched.

## Consequences

- Follow-on tasks T024q (QUIC channel), T024h (host sender rules).

## Agent log

- 2026-10-25 (UTC) · opencode-go/beam-free + T024 · Filed ADR-0028 Accepted per D35.
