# crpg-testkit — architecture

Shared test machinery, built once so it cannot fork across crates.

**State:** the hash-sequence harness is complete (T008b): scripted
scenario runs, golden-file write/compare, and the convention later tasks
use. The versioned replay is merged (T009a + T009c, 2026-09-07):
opaque-payload record/parse/validate/playback through public sim APIs, an
exact-tick divergence report reusing the harness mismatch, and per-target
checked-in replay goldens under the two-target policy below. Required native
Windows/MSVC and genuine WSL Ubuntu Linux/GNU gates passed (see the
[T009c completion record](../../tasks/T009c.md)); T009b's thin
`crpgc replay` wrapper in `crpg-cli` is next. Fixture
campaigns, contract conformance suites and further helpers are planned,
each owned by the task that first needs it.

Decisions: [ADR-0009](../adr/0009-determinism-scope.md) for the scope the
harness operates under; [ADR-0010](../adr/0010-testkit-mismatch-shape.md) for
the truthful mismatch shape; E005 for its one-way dependency direction.
ADR-0012/T009c supersedes only ADR-0009 Decision 3's Linux-only selection.
Working contract: [`crates/crpg-testkit/AGENTS.md`](../../crates/crpg-testkit/AGENTS.md).

---

## Position

`crpg-testkit` may reach any simulation crate except the Godot bridge — the
one crate in the ALLOWED table with an exclusion instead of a closed set.
Under E005's topology, a testkit that pulled the engine would pull it into
every higher crate using the helpers — which is why the bridge exclusion
exists. Today it
reaches `crpg-sim` (normal) and `crpg-core` (tests only), and no manifest
takes testkit yet. Later helpers add edges with their tasks: data fixtures
will want `crpg-data`, rules suites `crpg-rules`, each justified in its task
file.

The direction is one-way by E005: testkit reaches down through sim, while
core, data, rules, and sim never reach back, including through a
dev-dependency. Their self-contained tests stay in their own crates and their
cross-layer integration tests live here. This is an architecture boundary,
not a claim that Cargo cannot compile dev-only cycles.

## Modules

- **`harness` — `run_hash_sequence`, goldens, errors.** The fixed
  script→tick→hash interleaving, the line-hex golden format with `#` scope
  headers, and exact-tick mismatch reports carrying only present sides
  (ADR-0010). T009's replay and T016's combat are named future consumers;
  the module serves them, it does not anticipate them.
- **`replay` — `Replay`, playback, divergence.** The versioned `.replay`
  schema (format 1: seed, campaign/engine identity, tick count, ordered
  inputs with opaque `serde_json::Value` payloads), validation before
  playback in fixed check order, the input→tick→hash interleaving that
  keeps replay hashes comparable with harness hashes, and
  `play_and_verify`: the end-to-end gate that reuses `verify_golden` and
  attaches replay identity to the `Mismatch` as a boxed `ReplayDivergence`.
  Payload meaning stays with the caller (`ApplyInput` over public `World`
  APIs); no game-specific input vocabulary lives here. T009b's `crpgc
  replay` is the thin CLI consumer; T016's combat is the next behavioural
  one.

## Today versus planned

| Exists (T008b + T009a) | Planned (owner) |
|---|---|
| Hash sequences, golden write/compare, error shape | Fixture campaigns (first data task needing one) |
| Replay format + validation + playback + typed divergence | Contract conformance suites (spec §15.3, first backend) |
| Determinism/tamper/shape/replay-behaviour tests | — |
| Scope-header convention and independent Windows/MSVC and Linux/GNU golden gates (T009c, step 9; merged 2026-09-07) | T009b `crpgc replay` wrapper (`crpg-cli`) |

## Target-scoped replay policy

Windows `x86_64-pc-windows-msvc` is the primary development, product,
release-gating, and behavioural-baseline target. Linux
`x86_64-unknown-linux-gnu` is fully supported for dedicated servers, headless
CLI/tooling, server-side extensibility, and testing; failures are defects.
Windows supports client, editor, embedded single-player server, dedicated
server, and CLI. Linux GUI client/editor support is not promised, and Linux
headless support must not depend on Godot.

The portable input remains `crates/crpg-testkit/fixtures/replay_basic.replay`.
Under `crates/crpg-testkit/goldens/`, the independent baselines are:

- `replay_basic_rust-1.98.0_x86_64-pc-windows-msvc_test-default.golden`
- `replay_basic_rust-1.98.0_x86_64-unknown-linux-gnu_test-default.golden`

Real `play_and_verify` comparisons select their paths at compile time using
`cfg(all(target_os = "windows", target_arch = "x86_64", target_env = "msvc", debug_assertions))`
and
`cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu", debug_assertions))`,
respectively. Other targets/profiles retain portable parse/play repeatability
and shape coverage without reading either golden. Runtime platform selection,
tolerance, and cross-platform equality assertions are forbidden. Each target
must match its own baseline exactly; a missing baseline is `Io(NotFound)`,
not a skipped green check.

The pinned Rust 1.98.0 toolchain, complete target triple, normal test profile,
and default features define each scope; changing any element requires a
reviewed re-baseline. Each baseline is independently generated on its genuine
native target through `read_replay` -> `play_replay` -> `write_golden`, never
copied from the other. The genuine T009a Linux record is preserved; renaming
that artifact is not regeneration. T009c native re-verification passed as
recorded in its completion record.
Neither Windows' primary baseline nor Linux's server regression baseline
promises cross-platform lockstep. Replay API, format, comparison semantics,
and ADR-0009 hash-exclusion governance remain unchanged.

One reusable platform-neutral authoritative server implementation must serve
Windows embedded single-player, Windows dedicated, and Linux dedicated hosts.
In-process transport preserves the client/server authority boundary: clients
never mutate authoritative state directly. E012/E022 own the unresolved
package/API placement; E023 owns native-extension ABI/loading governance.
Platform-specific hosting stays above core/rules/sim. Future real product
builds and smoke tests activate with their capabilities under E020, not as
placeholder jobs.

## What consumers inherit

- **Comparable sequences.** Same interleaving everywhere means a hash at
  index *n* means the same thing in every task's test: the world after the
  *n*-th scripted step and tick.
- **Reviewable goldens.** Line-hex under scope headers diffs cleanly; a
  one-tick behaviour change shows as one changed line plus the report naming
  it.
- **Recordable behaviour.** A replay (seed + ordered opaque inputs) plays to
  the same hash sequence on the same build, so any consumer — T009b's CLI,
  T016's combat, future sweeps — regresses behaviour through one artifact
  and one gate instead of a private dialect each.
- **No engine in the test graph.** Depending on testkit never drags Godot,
  windowing, or audio into a headless test — the exclusion that keeps CI
  fast is structural, not careful.

## Agent log

- 2026-09-06 (UTC) · opencode/muse-spark + T008b · Wrote the crate doc for the harness: testkit's position and one-way direction, the module map, today-vs-planned with T009/T016/E020 as named consumers, and what consumers inherit.
- 2026-09-06 (UTC) · opencode/muse-spark + testkit mismatch hardening · Recorded ADR-0010, corrected the E005 dev-dependency claim to proposed-not-fact, and noted truthful mismatch sides.
- 2026-09-06 (UTC) · opencode/gpt-5.6-sol + E005 decision · Recorded the ratified lower-layer test ownership boundary and removed the incorrect Cargo-cycle justification.
- 2026-09-06 (UTC) · opencode/gpt-5.6-sol + E020 decision · Assigned canonical-Linux enforcement to T009a under the existing workspace-test job instead of planning a placeholder CI job.
- 2026-09-06 (UTC) · opencode/muse-spark + T009a · Recorded the replay module (opaque payloads, input→tick→hash ordering, boxed divergence), the checked-in canonical-Linux fixture pair, and CI step 9 going live with T009b/T016 as the named consumers.
- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c · Aligned the architecture with Windows-primary and fully supported Linux headless hosting and independent exact-build golden scopes. Distinguished unmerged implementation from pending native verification and deferred host/extension implementation decisions to their owners.
- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c verification alignment · Replaced active pending-verification wording with the reported native gate passes and linked the completion record. Kept T009c the next review/merge priority and T009b blocked on landing, without claiming either replay task committed or merged.
- 2026-09-07 (UTC) · opencode/big-pickle + T009a/T009c merged · Updated the state and table rows to the merged replay and two-target golden gates; T009b is next.
