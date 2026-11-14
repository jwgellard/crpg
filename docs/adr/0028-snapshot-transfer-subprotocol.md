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

## Host requirements (T024 S§11)

Authoritative contract: `tasks/T024.md` S§11.

- Transfer ids are strictly increasing per epoch, from 1, and never reused (need not be contiguous).
- `event_seq` never resets within an epoch: a snapshot's `cursor` is the last `event_seq` issued before capture, and later deltas are above it.
- In the peer's single total order, a `Begin` precedes every delta above its cursor, and a transfer's chunks follow its `Begin`.
- The host starts a new transfer only after `Published` or `Resync` for the previous one, or after sending `Cancel` for it.
- On revocation touching an in-flight transfer, the host sends `Cancel(Aborted)`, stops sending that id's chunks, and captures a new body under the new grants.
- A body holds only facts granted at capture: presence makes an entity, the health grant adds `health`, the `turn_state` grant adds `turn`, and `active` is `None` unless that entity is present.
- Deltas after capture wait in egress under the existing caps; overflow means `Cancel` and a fresh snapshot, or a fence — never a skipped delta.
- `plan_transfer` produces canonical frames; a host that encodes frames itself must produce the same bytes.

## Authority and affected surface

- Owning crate: `crates/crpg-net` modules `protocol_transfer`, `codec_transfer`, `reassembly`.
- Contract: `tasks/T024.md` S§0–S§16.

## Compatibility, hash and dependency impact

- No new dependency. v1/v2 untouched.

## Consequences

- Follow-on tasks T024q (QUIC channel), T024h (host sender rules).

## Supersession and corrections

- 2026-11-10: T140 (protocol v3, D52) adds a separate 0x82 snapshot transfer with a 4 MiB / 128-chunk cap; the 0x81 transfer and its 1 MiB / 32-chunk cap are unchanged for v1/v2.
- 2026-11-13: the v3 receiver is `ReassemblyPoolV3` in `crates/crpg-net/src/pool_v3.rs` (T140r1, D55): this ADR's reassembly and cutover gate transliterated for `0x82` frames and v3 deltas, sized by T140e's `TransferLimitsV3` (20 s deadline, 4 MiB per-receiver staging). `ReassemblyPool` (`reassembly.rs`) is unchanged and still serves only v1/v2 and `0x81`.

## Agent log

- 2026-10-25 (UTC) · opencode-go/beam-free + T024 · Filed ADR-0028 Accepted per D35.
- 2026-10-25 (UTC) · opencode-go/beam-free + T024-docs · Added Host requirements (T024 S§11) section listing each sender rule.
- 2026-11-10 (UTC) · claude-code/haiku-5.5 (bookkeeper) + S49-bookkeeping · Added the Supersession and corrections section the coordinator directed, recording the T140 0x82 transfer cap as a dated correction; the 0x81 transfer and its caps are unchanged.
- 2026-11-13 (UTC) · claude-code/claude-opus-5-5 (architect) + T141 contract brief · Appended the dated note that `pool_v3` (T140r1) is the v3 reassembly gate, the manager edit `tasks/T140r.md` Routing and size names, so readers of this ADR find the v3 receiver without the task files.
