# T040 — the dedicated `crpg-server` binary (`crpg-server`, `.github/workflows`)

**Status: blind draft A, not approved.** Written by a DeepSeek V4 Pro architect
from the fact sheet (`tasks/T040-facts.md`) and the binding inputs it cites:
ADR-0012 (`docs/adr/0012-windows-primary-platform.md`), D65a, D66b, D67a, the
root `AGENTS.md`, and the T040 row of `docs/IMPLEMENTATION_PLAN.md` (§3, M4
4E). A Claude architect writes the final contract; the user approves it.
Nothing here is approved. Any fact was re-checked in code; anything not
re-checked is cited only as "not verified" or left as an open question.

Citation convention (T140's): a bare `:N` or `:N–M` is a line of the file
cited earlier in the same item or sentence. Branch citations are
`<branch>:<path>:line`. Every citation was checked on `task/T040-draft-a-d1`
(from `task/T040-facts-f1`: `master` plus the fact sheet).

## Task

Turn `crpg-server`'s refuse-to-run binary into the dedicated server program.
Today `src/main.rs` prints `crpg-server: dedicated adapter not yet
implemented` and exits 1 (`crates/crpg-server/src/main.rs:9–12`); it is a
wiring shell over the `crpg_server` library, and no host policy lives in it
(`crates/crpg-server/AGENTS.md:24–26`). T040 replaces it with a real program
that, on Windows/MSVC and on a Godot-free Linux/GNU build, loads a
certificate and key, an invitation file, and a save, then hosts the T154 run
loop behind `QuicHosting` (`task/T154-c1:tasks/T154.md` Reading 13) and runs
it on the `runner` thread until a clean shutdown.

The pieces are already built and T040 only wires them (D66b, answer (a),
`tasks/DECISIONS-D66.md:17–25`): the run loop (`run.rs`), the clock and
thread (`runner.rs`), the QUIC adapter (`quic.rs`), the save adapter
(`save.rs`), and T154d's `QuicHosting`/`QuicAiHook` (contract only, not yet
built). What T040 adds is the operator surface the library deliberately
leaves out (`docs/architecture/crpg-server.md:26–33`): the command line, a
JSON configuration file, certificate/key loading, invitation registration,
the save identity and a never-decreasing incarnation source, logging — with
the D67a rejected-message log line and a "where do server logs go" decision
(`tasks/BACKLOG.md:341`, `tasks/DECISIONS-D67.md:5–12`) — and graceful
shutdown. It also wires the two filed follow-ups: `bind_area_bounds` at area
load and `place` after transfers and spawns (T037 Reading 5,
`tasks/T037.md:160–167`, `tasks/BACKLOG.md:344`), and the gate-12 dedicated
server artifact jobs (server half of `docs/CRPG_ENGINE_SPEC.md:1333`,
Windows plus Godot-free Linux, `:1345–1348`).

T040 is one crate (`crpg-server`) plus the workflow file, in two parts
below. No new dependency is added: argument parsing is hand-rolled over
`std::env::args`, configuration is JSON through the already-present
`serde_json` (`crates/crpg-server/Cargo.toml:32–33`), and logging is a
std-only line writer over `std::io`. A std-only alternative is given for
every place a parser or logger library would otherwise be tempting
(`AGENTS.md` "Do not add dependencies without being asked").

## Parts

Two parts, both S-set. T040a is S-set because the T040 row is `C → I, S`
(`docs/IMPLEMENTATION_PLAN.md:245`); T040b is S-set because
`.github/workflows/**` is in the serialized set (`docs/IMPLEMENTATION_PLAN.md:96`).
Order: **T040a then T040b** — T040b's artifact job builds and uploads the
binary T040a produces, so the job name and the binary name must be stable
before the workflow lands. T040a may run alone; T040b starts after T040a
merges.

| Part | Crate / tree | Scope | S-set |
|---|---|---|---|
| T040a | `crpg-server` | the binary: CLI, config, certificate/key, invitations, save + incarnation, logging (the D67a line and its destination), shutdown, hosting the T154 loop, `bind_area_bounds`/`place` wiring | **yes** (T040 row) |
| T040b | `.github/workflows/ci.yml` | the gate-12 dedicated-server artifact jobs (Windows/MSVC and Linux/GNU), no Rust | **yes** (workflows) |

True prerequisite: T154a–d merged (`QuicHosting`, `runner`, the loop), T039
(save), T023b (QUIC adapter), T153 (`Host::drive_ai`), T150c
(`QuicHost::advance`, `register_v4_invitation`). The plan's stale
`T023b` prerequisite (`docs/IMPLEMENTATION_PLAN.md:245`) predates the T154
run loop; the handoff is `task/T154-c1:tasks/T154.md:245–252`.

### T040a owns
- `crates/crpg-server/src/main.rs` (replaced: the entry, wiring and shutdown)
- `crates/crpg-server/src/cli.rs` (new, binary-only: `std::env::args` parsing)
- `crates/crpg-server/src/config.rs` (new, binary-only: JSON config load and
  validation over the existing `serde_json` edge)
- `crates/crpg-server/src/logline.rs` (new, binary-only: the std-only log
  writer and its line formats)
- `crates/crpg-server/src/quic.rs` (one additive item: `QuicHost::last_report`,
  Reading 8; a Manager edit to T023b/T154d's surface lists)
- `crates/crpg-server/tests/dedicated_binary.rs` (new: black-box binary cases
  and unit cases for the pure `cli`/`config`/`logline` parts)
- `docs/architecture/crpg-server.md` (the "planned and **not** here" list
  shrinks), `crates/crpg-server/AGENTS.md` (invariant 7 fence amended, the
  binary surface named, new traps)
- goldens: none new, none moved.

### T040b owns
- `.github/workflows/ci.yml` (one new `server-artifacts` job; no other
  workflow file is created or touched)
- goldens: none new, none moved.

## Readings

## Interface

## Behaviour

## Errors

## Tests

## What must not change

## Dependency edges

## Definition of done

## Out of scope

## Open questions

## Manager edits

## Agent log
