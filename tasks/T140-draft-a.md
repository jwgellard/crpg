# T140-draft-a — blind draft: the next `crpg-net` protocol version (v3)

Status: **draft for adjudication, 2026-11-10.** Not approved. A Claude architect
adjudicates it against the other blind draft; the user approves the result.
Nothing here is a claim about code that exists.

## Task

Draft the exact contract for T140: `crpg-net` gains protocol version 3, which
carries ADR-0029 N1's world facts, host facts, the three N5 events, the six
intents, the rejection codes T138/T139/T137 left to T140, a v3 snapshot body,
per-field grants for every new fact, and the N4 `Clock` heartbeat. v1, v2,
the `0x81` transfer subprotocol and the `0x91` resume subprotocol stay frozen
byte for byte, and `crpg-net` still reads no `World`.

## Crate(s)

Sole crate: `crpg-net`. Nothing in `crpg-core`, `crpg-data`, `crpg-rules`,
`crpg-sim` or `crpg-server` changes; T141 owns the `crpg-server` projection.
`splits:` three implementer runs, in dependency order.

T140a — protocol and codec (owns `crates/crpg-net/src/protocol_v3.rs`,
`crates/crpg-net/src/codec_v3.rs`, `crates/crpg-net/tests/codec_v3.rs`).
`goldens:` no existing golden moves; `codec_v3` adds literal v3 intent/delta
goldens. v1/v2 goldens must not move because the new module never imports or
edits `protocol.rs`/`protocol_v2.rs`/`codec.rs`/`codec_v2.rs`.

T140b — event and fact projection (owns
`crates/crpg-net/src/projection_v3.rs`, `crates/crpg-net/src/interest_v3.rs`,
`crates/crpg-net/tests/events_v3.rs`, `crates/crpg-net/tests/interest_v3.rs`).
`goldens:` `tests/events_v2.rs` and `tests/interest.rs` literal pages must not
move; v3 adds new literal pages in its own suites.

T140c — v3 snapshot transfer and docs (owns
`crates/crpg-net/src/protocol_transfer_v3.rs`,
`crates/crpg-net/src/codec_transfer_v3.rs`,
`crates/crpg-net/tests/snapshot_transfer_v3.rs`,
`docs/architecture/crpg-net.md` and `crates/crpg-net/AGENTS.md`). `goldens:`
`tests/snapshot_transfer.rs` (`0x81`), `tests/snapshot_transfer_contract.rs`
and `tests/reconnect_wire.rs` (`0x91`) must not move; v3 adds its own literal
`0x82` frames and a body-size golden.

## Readings of the ADR

1. One new main protocol version, byte 3.
2. New tags start at 11; v2 vocabulary rides `Legacy` with no extra bytes.
3. `Turn` is replaced by one richer `Encounter` op in v3.
4. The timeline is its own op, ascending, with raw `i32` keys.
5. Per-entity facts are one `EntityFacts` op, not five ops.
6. Bounds are small enough that a maximal v3 body fits 1 MiB.
7. `RejectionCode` is versioned; v3 has its own enum and `Receipt` op.
8. The new codes are exactly the union the other contracts left.
9. The v3 snapshot is transfer subprotocol `0x82` on lane 2.
10. Snapshot and delta facts match; host facts are not snapshot facts.
11. `Joined` and `Left` get notice tags; `Moved` does not.
12. The heartbeat is a delta op every 100 ticks and on pause change.
13. Host facts are one `HostFacts` op, sent on change and to every new viewer.
14. v3 re-declares the intent vocabulary; tags 0 and 1 keep v1 payloads.
15. `crpg-net` only encodes host-supplied values; T141 reads `World`.
16. Roll transparency is not opened by this draft.

## Interface

To be filled.

## Behaviour

To be filled.

## Errors

To be filled.

## Tests

To be filled.

## Out of scope

To be filled.

## Definition of done

To be filled.

## Questions for the user

To be filled.

## Agent log

- 2026-11-10 (UTC) · deepseek-v4.1-flash-free + T140 blind draft a · Skeleton created with every required heading before filling, per the brief's size rule.
