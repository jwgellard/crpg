# ADR-0028: Snapshot transfer subprotocol

Status: Accepted

## Context
T024 adds a separate snapshot-transfer subprotocol to `crpg-net` (D11, D35).

## Decision
- Separate transfer subprotocol, first byte `0x81`, lane 2, downstream tags 0x10-0x12, upstream 0x20-0x21.
- Packing: 1-8 segments of 4096 bytes per chunk.
- Two layers: byte-layer `TransferAssembly` proves full 1 MiB / 32-chunk envelope; `ReassemblyPool` adds deadline, budgets, body validation, atomic publication, delta cutover.
- Cutover by delivery cursor; fail closed.
- No integrity digest.

## Links
Contract: tasks/T024.md S§2 and S§13.

- 2026-10-25 (UTC) · opencode-go/beam-free + T024 · ADR filed.