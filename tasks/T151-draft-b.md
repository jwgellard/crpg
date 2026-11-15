# T151-draft-b — blind contract draft: `crpg-godot` bridge move to protocol v4

> Blind draft. Nothing here is approved: a Claude architect writes the final
> contract and the user approves it. Built after T037d from
> `tasks/T151-facts.md`; every `path:line` was re-read in this slot. A bare
> `:N` is a line of the file cited earlier in the same item.

## Task

Move the `crpg-godot` bridge from its v2 session to a protocol v4 session so
a Godot client sees positions and sends moves through the embedded server.
Today `BridgeCore::open` builds the host with
`HostConfig { protocol: ProtocolSelection::V2, incarnation: 1 }`
(`crates/crpg-godot/src/core/mod.rs:302`), the client half holds one v2
`ReplicaWorld` (`crates/crpg-godot/src/core/client.rs:292`) decoding with
`codec_v2::decode_delta` (`crates/crpg-godot/src/core/client.rs:322`), and the
view has no position member (`crates/crpg-godot/src/core/client.rs:28–45`)
because the v1/v2 wire has "no transform op"
(`crates/crpg-net/src/protocol.rs:305`). After this task the bridge binds
with the T037 future `Host::bind_v4_peer` (`tasks/T037.md:439–441`), the
client half feeds `ReassemblyPoolV4::pop_update`
(`crates/crpg-net/src/pool_v4.rs:656`) into `ReplicaWorldV4`
(`crates/crpg-net/src/replica_v4.rs:55`) plus `MoveOutbox`
(`crates/crpg-net/src/outbox_v4.rs:54`), and GDScript reads granted positions
and sends `MoveTo`/`Halt` through new polled getters and one new submit call.
T049 interpolation (G1), T026 prediction (G2) and T050 click-to-move (G3) are
later tasks: this one exposes the data and the send path without doing them
(`tasks/T026p.md:1477–1479`; `tasks/T037.md:1356–1358`).

## Parts

Order: **serial** A → B → C → D. Each part is one crate (or client) and one
implementer run. Parts A and B stay below the GDExtension boundary and are
not S-set (no wire byte, version or tick order is fixed here; v4 bytes are
T026p's). Part C touches the GDExtension boundary
(`docs/IMPLEMENTATION_PLAN.md:86–98`, "the GDExtension boundary in
`crpg-godot`" at `:89–95`) and is S-set: one at a time and human-reviewed.
Part D is GDScript only, a separate part per the brief, not S-set.

| Part | Scope | Size (source + tests) | S-set | Review |
|---|---|---|---|---|
| T151a | `crpg-godot` core client half: pool, replica, outbox, view, submit | ~200 + ~400 | no | reviewer, one fix round |
| T151b | `crpg-godot` core authority half: v4 bind, samples, receipts, hand-off | ~150 + ~350 | no | same |
| T151c | gdext shell: new `#[func]`s and view getters | ~80 + ~120 | **yes** (boundary) | same + human |
| T151d | `apps/client` GDScript: read positions, send moves, runner cases | GDScript + cases only | no | same |

### T151a owns

- `crates/crpg-godot/src/core/client.rs`
- `crates/crpg-godot/src/core/client/tests.rs`
- `crates/crpg-godot/src/core/tests.rs` (new v4 cases only)
- `docs/architecture/crpg-godot.md`, `crates/crpg-godot/AGENTS.md`
- goldens: none new, none moved.

### T151b owns

- `crates/crpg-godot/src/core/authority.rs`
- `crates/crpg-godot/src/core/mod.rs` (bind + wiring only)
- `crates/crpg-godot/src/core/authority/tests.rs`
- `docs/architecture/crpg-godot.md`, `crates/crpg-godot/AGENTS.md`
- goldens: none new, none moved.

### T151c owns

- `crates/crpg-godot/src/shell.rs` (new `#[func]`s and getters only)
- `crates/crpg-godot/tests/bridge.rs` (public-API cases only)
- `docs/architecture/crpg-godot.md`
- goldens: none new, none moved.

### T151d owns

- `apps/client/scripts/Main.gd`, `apps/client/scripts/SceneSync.gd`
- `apps/client/tests/scene_sync_cases.gd`, `apps/client/tests/smoke.gd`
- `docs/architecture/crpg-godot.md`
- goldens: none new, none moved.

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
