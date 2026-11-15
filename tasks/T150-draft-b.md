# T150-draft-b — lane-1 movement transport (`crpg-net-quic` + `crpg-server` route)

**Status: draft, blind (slot 2, session 60). Nothing here is approved.**
Draft of the exact contract for T150, from the approved T026p contract
(`tasks/T026p.md:1471–1476`), the approved T037 contract
(`tasks/T037.md:1351–1355`), ADR-0036, ADR-0024, ADR-0025, ADR-0026 and
`tasks/DECISIONS-2026-09-30.md:83–88`, built on `tasks/T150-facts.md`.
A Claude architect writes the final contract; the user approves it.

## Task

Carry lane 1 over QUIC. `crpg-net-quic` gains a second reliable ordered
stream pair for lane-1 intents upstream (`MoveTo`/`Halt`) and receipts
downstream, the QUIC DATAGRAM extension for `MoveSample`s with a negotiated
floor of `MAX_MOVE_DOWN_BYTES` (1,024, refused at connect otherwise), D23's
vendored dedup patch through the T023v mechanism, and the native Windows
impairment-probe re-run as evidence. The `crpg-server` `quic.rs` route binds
v4 sessions and pumps lane-1 bytes through the T037 surface it calls:
`bind_v4_peer`, `ingest_move`, `take_move_receipts`, `take_samples`
(`tasks/T037.md:437–441`, `:483–484`, `:487`, `:462`).

Built after T037d, so the T037 Interface is the host surface it calls;
`quic.rs` is otherwise frozen (`tasks/T037.md:1338`, `:1351–1355`). One task
= one crate per part (`AGENTS.md:57`; the scope names two crates,
`tasks/T037.md:1351–1355`), so three serial parts: **T150a** the vendored
patch (S-set), **T150b** the transport (not S-set), **T150c** the server
route (not S-set). S-set here is `docs/IMPLEMENTATION_PLAN.md:1.4`'s
serialized set: a vendored dependency patch, `deny.toml` and the workspace
`Cargo.toml` are S-set; nothing else in this draft is.

## Parts

TODO (fill)

## Readings

TODO (fill)

## Interface

TODO (fill)

## Behaviour

TODO (fill)

## Errors

TODO (fill)

## Tests

TODO (fill)

## What must not change

TODO (fill)

## Dependency edges

TODO (fill)

## Out of scope

TODO (fill)

## Open questions (each with a recommendation)

TODO (fill)

## Agent log

- 2026-11-15 (UTC) · opencode-go/muse-spark-1.3-contributor + T150 draft b · Opened the blind-draft skeleton with every contract section so each section can be filled and committed separately.
