# T151 — `crpg-godot` bridge move to protocol v4 (`crpg-godot`, `apps/client`)

**Status: draft (blind), 2026-11-15 (UTC).** Not approved. A Claude architect
writes the final contract; the user approves it. Bound by ADR-0033, ADR-0036,
the approved T026p contract (`tasks/T026p.md` §Out of scope `:1477–1479`) and
the approved T037 contract (`tasks/T037.md` §Out of scope `:1356–1358`, and the
Interface items T037b–T037d). Citation convention (T037's): a bare `:N` or
`:N–M` is a line of the file cited earlier in the same list item or sentence.
Every `path:line` was checked on `2aae6c9` (branch `task/T151-draft-a-d1`).

## Task

Move the `crpg-godot` bridge from protocol v2 to protocol v4, so a Godot client
sees positions and sends moves through the embedded single-player server. The
three `crpg-net` client types replace the v2 ones: `ReassemblyPoolV4`
(`crates/crpg-net/src/pool_v4.rs:102`) gates and reassembles delivery,
`ReplicaWorldV4` (`crates/crpg-net/src/replica_v4.rs:55`) holds rows, positions
and samples, and `MoveOutbox` (`crates/crpg-net/src/outbox_v4.rs:54`) is the
lane-1 sender. The session is created choosing v4 with T037's `bind_v4_peer`
(`tasks/T037.md:439–441`); positions and `Moved` reach GDScript as polled view
state; a move request leaves GDScript as a lane-1 intent and its
accept/refuse/supersede receipt is polled back. T049 interpolation (G1), T026
prediction (G2) and T050 click-to-move (G3) are later tasks; this one leaves
room for them without doing them.

Today the bridge opens every session with `HostConfig { protocol:
ProtocolSelection::V2, incarnation: 1 }` (`crates/crpg-godot/src/core/mod.rs:302`),
decodes whole v2 frames itself into a v2 `ReplicaWorld`
(`crates/crpg-godot/src/core/client.rs:18`, `:292`, `:321–329`), submits with
`codec_v2::encode_intent` (`:523`), and treats a `DeliveryKind::Transfer` entry
as a fatal fence (`crates/crpg-godot/src/core/authority.rs:61–75`). None of
those survive the move. The GDExtension boundary is S-set
(`docs/IMPLEMENTATION_PLAN.md:95`); only the part that adds FFI members touches
it.

## Parts

## Readings

## Interface

## Behaviour

## Errors

## Tests

## What must not change

## Dependency edges

## Out of scope

## Open questions

## Agent log

- 2026-11-15 (UTC) · opencode-go/deepseek-v4-pro + T151 draft a · Blind draft of the `crpg-godot` bridge move to protocol v4 (`ReassemblyPoolV4`, `ReplicaWorldV4`, `MoveOutbox`), so the bridge opens a v4 session, sees positions and `Moved`, and sends lane-1 moves through the embedded server, leaving T049/T026/T050 room without doing them.
