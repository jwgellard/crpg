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

Two parts, one crate (`crpg-godot`), order **serial**: a → b. a is the
Godot-free core; b is the gdext shell. `apps/client` GDScript is **not** changed
(Reading 4), so no third part.

| Part | Crate | Scope | S-set | Review |
|---|---|---|---|---|
| T151a | `crpg-godot` | v4 session creation, the pool + replica + outbox swap, positions in the internal view | no | Claude reviewer mutant run, one fix round |
| T151b | `crpg-godot` | position and move FFI members on `CrpgView`/`CrpgSession` | **yes**: touches the GDExtension boundary (`docs/IMPLEMENTATION_PLAN.md:95`) | human-reviewed (S-set) |

### T151a owns
- `crates/crpg-godot/src/core/mod.rs`, `crates/crpg-godot/src/core/client.rs`,
  `crates/crpg-godot/src/core/authority.rs`, `crates/crpg-godot/src/core/errors.rs`
- `crates/crpg-godot/src/core/tests.rs`,
  `crates/crpg-godot/src/core/{authority,client,errors,link}/tests.rs`
- `crates/crpg-godot/tests/bridge.rs`, `crates/crpg-godot/tests/source_rules.rs`
- `docs/architecture/crpg-godot.md`, `crates/crpg-godot/AGENTS.md`
- goldens: none new, none moved.

### T151b owns
- `crates/crpg-godot/src/shell.rs`
- `docs/architecture/crpg-godot.md`, `crates/crpg-godot/AGENTS.md`
- goldens: none new, none moved.

`core/link.rs` is owned by neither part unless T151a finds it must change; the
link stays two byte FIFOs (`crates/crpg-godot/src/core/link.rs:19–26`) and only
its callers learn to route more frame kinds (Reading 2). `Cargo.toml`,
`Cargo.lock`, `apps/client/**`, `ci.yml`, `deny.toml` and `rust-toolchain.toml`
are not in either part's owned list.

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
