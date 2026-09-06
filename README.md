# CRPG Engine

A purpose-built open CRPG engine and campaign editor: a deterministic Rust
simulation core with **no game-engine dependency**, a headless authoritative
server, and **Godot used only as a replaceable presentation host** for the
client and editor.

The guiding decision: **do not fork Godot.** Consume it as a version-pinned
presentation host through GDExtension, and build a fully Godot-free simulation
core. If Godot ever disappoints, replacing it is a client rewrite — the rules,
campaign format, netcode, AI, and server are untouched.

> **Status:** early development (Phase 1 — core skeleton and test harness).
> `crpg-core` has the planned Phase 1 primitives: entity identity, fixed-point
> math, named deterministic RNG streams, time counters, ULIDs, runtime string
> interning, and core errors. Every other crate is still a stub. See
> [docs/PROJECT_STATE.md](docs/PROJECT_STATE.md).

---

## Architecture

The whole project is a Rust workspace. The dependency direction is strictly
downward and enforced by CI — a cycle is a build failure, not a code-review
comment.

```
Presentation (Godot 4)          crpg-client, crpg-editor
      │
      │ GDExtension (godot-rust), narrow FFI surface
      ▼
         crpg-net   protocol, codec, QUIC transport, interest management
         crpg-sim   world store, systems, tick, movement, encounters
         crpg-ai  │ crpg-script │ crpg-persist
         crpg-rules   stats, modifiers, effects, resolution, actions
         crpg-data   campaign schema, serde, validation, migration
          crpg-core   ids, fixed-point math, RNG, time, event queue, errors
```

There are three shipped binaries plus a CLI:

| Binary | Contains | Renders? | Authoritative? |
|---|---|---|---|
| `crpg-server` | core, rules, sim, script, AI, net, persistence | No | **Yes** |
| `crpg-client` | Godot host + core in replica mode + net client | Yes | No |
| `crpg-editor` | Godot host + core in edit mode + privileged net client | Yes | No |
| `crpgc` | validate / migrate / pack / run / replay / diff | No | n/a |

Single-player runs the same authoritative server in-process. There is no
separate "single-player code path."

## Workspace layout

| Crate | Role |
|---|---|
| `crpg-core` | Core types: ids, fixed-point math, deterministic RNG, time, event queue, errors |
| `crpg-data` | Campaign schema, serde, validation, migration |
| `crpg-rules` | Rules kernel: stats, modifiers, effects, resolution, actions |
| `crpg-sim` | Simulation engine: world store, systems, tick, encounters, SimEvent stream |
| `crpg-ai` | AI logic |
| `crpg-nav` | Navigation / pathfinding |
| `crpg-net` | Networking, protocol, transport |
| `crpg-script` | Scripting (Lua event handlers, event IR) |
| `crpg-server` | Headless authoritative server binary |
| `crpg-cli` | CLI binary (`crpgc`) |
| `crpg-godot` | Godot integration (the **only** crate allowed `godot` and `unsafe`) |
| `crpg-edit` | Editor document model / tooling |
| `crpg-persist` | Persistence / save-load backends |
| `crpg-contracts` | Shared contracts |
| `crpg-testkit` | Test utilities |

## Key design decisions

- **Godot is pinned, not forked** — consumed as an unmodified presentation host
  via GDExtension. Any necessary engine patches stay a small patch queue against
  a pinned tag. See [ADR-0001](docs/adr/0001-godot-pinned-not-forked.md).
- **Rust below the presentation layer** — everything that runs authoritatively
  (core, rules, sim, server) is Rust with no Godot dependency. GDScript is
  reserved for client/editor view code *only*, because the server has no Godot
  and any rule written in GDScript could not run authoritatively. See
  [ADR-0002](docs/adr/0002-rust-for-the-core.md).
- **Deterministic simulation** — same binary + same inputs ⇒ same result, so
  replays, saves, and testing are first-class. Scope is replay, not lockstep
  ([ADR-0009](docs/adr/0009-determinism-scope.md)). Backed by lints that ban
  `HashMap` iteration and floating-point in the rules/sim paths.
- **Toolchain:** Rust 1.98.0 (see `rust-toolchain.toml`), edition 2021.

## Getting started

```sh
cargo build            # build the whole workspace
cargo test --workspace # run all tests
```

Before finishing work on a crate — the same gates CI runs, so a clean run
here is the whole check:

```sh
cargo fmt --all
cargo clippy -p <crate> --all-targets -- -D warnings
cargo test -p <crate>
python tools/lint/deps.py
python tools/lint/determinism.py
python -m unittest discover -s tools/lint -p "test_*.py"
```

CI (`.github/workflows/ci.yml`) runs `fmt`, `clippy` and tests across the
workspace on Linux and Windows, all with `--locked` so the tested dependency
graph is the committed one. On Linux it also runs `cargo deny`, the
dependency-direction architecture lint (`tools/lint/deps.py`), the determinism
lint (`tools/lint/determinism.py`), and the self-tests for both lints:

```sh
python -m unittest discover -s tools/lint -p "test_*.py"
```

## Development rules

These are non-negotiable and loaded by agents every session ([AGENTS.md](AGENTS.md)):

- Only `crpg-godot` may depend on `godot`. Only `crpg-godot` may use `unsafe` —
  every other crate root carries `#![forbid(unsafe_code)]`, and the
  architecture lint fails if one does not.
- Dependency direction: `core <- data <- rules <- sim <- {net, ai, script} <- server`.
  Never import upward. Never create a cycle. Dev-, build- and target-specific
  dependencies all count, and a renamed dependency is still the crate it names.
  That sentence covers the spine; the complete, enforced edge list is the
  `ALLOWED` table in [`tools/lint/deps.py`](tools/lint/deps.py).
- No `HashMap`/`HashSet` in `crpg-core`, `crpg-rules` or `crpg-sim`; use
  `IndexMap`/`BTreeMap`. No `f32`/`f64` in `crpg-core` or `crpg-rules`; use
  integers or `Fx16_16`. `crpg-sim` may use `f32` for spatial positions only
  (spec §2.4), never in a rules path.
- Do not modify `crpg-contracts` or `rust-toolchain.toml`.
- Do not weaken or delete an existing test to make a build pass.
- One task = one crate.

## Documentation

- [docs/CRPG_ENGINE_SPEC.md](docs/CRPG_ENGINE_SPEC.md) — the technical specification and roadmap
- [docs/PROJECT_STATE.md](docs/PROJECT_STATE.md) — living status file
- [docs/HANDOFF.md](docs/HANDOFF.md) — how to continue work
- [docs/architecture/](docs/architecture/) — per-crate design docs (spec §15.6: a crate without one is not ready for agent work)
- [docs/adr/](docs/adr/) — architecture decision records

## License

`MIT OR Apache-2.0`, at your option — as declared in the workspace
`Cargo.toml` and enforced for dependencies by [`deny.toml`](deny.toml).
Full texts: [LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE).

## Agent log

- 2026-09-05 (UTC) · opencode/muse-spark + E001/ADR-0008 · Core role now says event queue (generic substrate), sim role names the SimEvent stream; no other rows touched.
- 2026-09-06 (UTC) · opencode/muse-spark + E009 · ASCII layer diagram now says event queue in core, matching the role table; no other rows touched.
