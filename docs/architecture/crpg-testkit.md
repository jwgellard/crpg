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
`crpgc replay` wrapper in `crpg-cli` is merged (2026-09-07; see
[tasks/T009b.md](../../tasks/T009b.md)). Fixture
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
| Scope-header convention and independent Windows/MSVC and Linux/GNU golden gates (T009c, step 9; merged 2026-09-07); T009b `crpgc replay` wrapper (merged 2026-09-07) | T016 headless combat (next replay consumer) |

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

## Combat replay (T016c, tests-only)

The first behavioural replay consumer after T009b's CLI is T016c's
headless combat suite. `tests/support/combat.rs` (included directly from
`tests/combat_replay.rs`, never from `src/`) owns the production content
loader — repo-root `campaigns/fixtures/combat_basic/` through
`load_campaign` with engine `0.1.0`, asserting the 14-file map, the 11-id
index, empty `validate`, and canonical-source parity with
`rulesets/minimal-d6/` — and the caller-owned combat adapter that decodes
the versioned `{"combat": 1, "op": "init" | "attack"}` grammar onto
`start_encounter` / `perform_action` with strict unknown-field rejection.
`EncounterSpec` reference maps are assembled transiently per initialization
from the owned `LoadedCampaign`, never as self-referential state; authored
placement ULIDs bind to runtime entities by walking `world.combatants()` in
insertion order against `encounter.participants` in authored order.
Observation is behavior-free: shared `&World` reads plus read-only
`serde_json::to_vec(&world)` snapshots after every accepted attack and
detached `World` clones for terminal, trailing-tick suffix, and death-event
assertions (the `EventQueue` exposes no read iterator). The portable input
is `fixtures/combat_basic.replay`; the independent native baselines are
`goldens/combat_basic_rust-1.98.0_x86_64-pc-windows-msvc_test-default.golden`
and `goldens/combat_basic_rust-1.98.0_x86_64-unknown-linux-gnu_test-default.golden`,
each generated on its genuine native target through the production
`read_replay` → `play_replay` → `write_golden` path. The generic harness
and replay stay payload-agnostic; T016d writes its own CLI-owned adapter
against the same grammar.

### Contract notes moved from AGENTS.md (A0-8)

Moved from `crates/crpg-testkit/AGENTS.md` (A0-8): the cross-layer combat
proof owns no `src/` change and no public export:
`tests/support/combat.rs` (caller-owned adapter, content loader, trace
oracle helpers) included directly from the reserved `tests/combat_replay.rs`
suite via `#[path = "support/combat.rs"] mod combat;` so existing replay
suites and `tests/support/mod.rs` stay unchanged. The adapter loads
repo-root `campaigns/fixtures/combat_basic/` through production
`load_campaign` (engine `0.1.0`) from `CARGO_MANIFEST_DIR`-relative I/O,
binds authored placement ULIDs to runtime entities by walking
`world.combatants()` against `encounter.participants` in authored order,
and decodes the versioned `{"combat": 1, "op": "init" | "attack"}` grammar
with strict unknown-field rejection and the T016c §Payload check order.
Gameplay goes only through `start_encounter` / `perform_action`; trace and
terminal facts are collected with shared `&World` reads plus read-only
`serde_json::to_vec(&world)` snapshots and detached `World` clones (the
`EventQueue` has no read iterator, and draining the authoritative queue
would mutate hashed state). The portable input is
`fixtures/combat_basic.replay`; the scoped baselines are
`goldens/combat_basic_rust-1.98.0_x86_64-pc-windows-msvc_test-default.golden`
and `goldens/combat_basic_rust-1.98.0_x86_64-unknown-linux-gnu_test-default.golden`,
each generated independently on its genuine native target through
`read_replay` -> `play_replay` -> `write_golden` with a temporary
deleted-before-finish helper. T016d owns the CLI adapter; this adapter
never ships.

## Second-ruleset replay (T017e, specified before source)

The second behavioural replay consumer is T017e's srd suite.
`tests/support/srd.rs` (included directly from `tests/srd_replay.rs`,
never from `src/`) owns the `combat_srd` production content loader —
repo-root `campaigns/fixtures/combat_srd/` through `load_campaign` with
engine `0.1.0`, asserting the 17-file map, the 14-id index, empty
`validate`, and canonical-source parity with `rulesets/srd-lite/` — and the
caller-owned srd adapter that decodes the versioned `{"combat": 1}` grammar
extended with the `end` op onto `start_encounter` / `perform_action` with
strict unknown-field rejection. Per-attack trace entries carry faces,
modified totals, margins, outcomes, damage, both pool balances, attach
state, and the turn holder; `end` inputs advance with no trace entry.
Observation stays behavior-free (shared reads, read-only snapshots,
detached clones). The portable input is `fixtures/combat_srd.replay` (a
pinned ascending-probe seed with `K <= 12` actions plus `T = 4` trailing
ticks); the independent native baselines are the two
`goldens/srd_lite_rust-1.98.0_*_test-default.golden` files, each generated
on its genuine native target. B5 writes its own CLI-owned proof against the
same grammar.

As-built: seed 285 with `K = 9` and `TOTAL_TICKS = 14`; the 13-test
`srd_replay` suite pins the artifact, oracles, continuation, terminal, and
per-target goldens; `combat_basic` artifacts stay byte-identical.

### Contract notes moved from AGENTS.md (A0-8)

Moved from `crates/crpg-testkit/AGENTS.md` (A0-8): same tests-only shape
for `combat_srd`: `tests/support/srd.rs` (srd-only adapter, content
loader, trace oracle helpers) included directly from the reserved
`tests/srd_replay.rs` suite via `#[path = "support/srd.rs"] mod srd;` so
`support/combat.rs`, `tests/combat_replay.rs`, and every `combat_basic`
artifact stay unchanged. The adapter loads repo-root
`campaigns/fixtures/combat_srd/` through production `load_campaign`
(engine `0.1.0`) and decodes the versioned `{"combat": 1}` grammar
extended with the `end` op
(`{"combat": 1, "op": "end", "actor": <ULID>}`) under the same strict
unknown-field rejection. Accepted inputs only (`init`, `attack`, `end`);
rejections are covered by direct-adapter rollback tests, never replay
inputs. The portable input is `fixtures/combat_srd.replay` (pinned
ascending-probe seed, `K <= 12` actions plus `T = 4` trailing ticks); the
scoped baselines are
`goldens/srd_lite_rust-1.98.0_x86_64-pc-windows-msvc_test-default.golden`
and
`goldens/srd_lite_rust-1.98.0_x86_64-unknown-linux-gnu_test-default.golden`,
each generated independently on its genuine native target through
`read_replay` -> `play_replay` -> `write_golden` with a temporary
deleted-before-finish helper. B5 owns the CLI proof (including the
CLI-side `end` decode); this adapter never ships.

As-built: seed 285, `K = 9`, `TOTAL_TICKS = 14` (terminal prefix at tick 9
with the hero dead and the goblin surviving at 1; alternate seed 286 with a
different tick-1 focus face); 13-test `srd_replay` suite green on both
natives with per-target goldens byte-stable; `combat_basic` replay,
goldens, and suites pass unmodified.

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

## Public API (changing this requires an ADR)

Moved from `crates/crpg-testkit/AGENTS.md` (A0-8): `run_hash_sequence`,
`ScriptStep`, `write_golden`, `verify_golden`, `Mismatch`, `HarnessError`,
and the replay surface: `Replay`, `ReplayInput`, `ApplyInput`,
`ReplayDivergence`, `ReplayError`, `REPLAY_FORMAT_VERSION`,
`MAX_REPLAY_TICKS`, `MAX_REPLAY_INPUTS`, `Replay::new`, `validate_replay`,
`play_replay`, `write_replay`, `read_replay`, `play_and_verify`.

- `run_hash_sequence(seed, ticks, script) -> Vec<[u8; 32]>`: script step,
  then `tick`, then `state_hash`, every tick, from `World::new(seed)`.
- `ScriptStep = Box<dyn FnMut(&mut World)>`: public sim API only — a script
  needing a private seam proves the sim API is missing something.
- `write_golden(path, hashes)`: lowercase hex, one per line, `#` scope
  headers, trailing newline, parents created.
- `verify_golden(path, hashes) -> Result<(), HarnessError>`: `Ok` on full
  match; `Mismatch` enum on divergence — `Diverged`, `GoldenShort`,
  `RunShort`, `Malformed`, `InvalidUtf8` (ADR-0010, only present sides);
  `Io` on filesystem failure. Headers never compare.
- `Replay { format_version, seed, campaign_id, campaign_version,
  engine_version, total_ticks, inputs }`: the recorded run. Identity is
  recorded, never enforced — playback does not compare versions against the
  running build. `Replay::new` validates; deserialized replays are
  validated by every consuming entry point.
- `ReplayInput { tick, payload }`: one scheduled input. `tick` is the
  zero-based tick index the payload applies before; the payload is an opaque
  `serde_json::Value` testkit never interprets.
- `ApplyInput = Box<dyn FnMut(&mut World, &Value) -> Result<(), String>>`:
  caller-owned payload meaning over public `World` APIs only. `Err(reason)`
  aborts with `ApplyFailed` at that tick and index.
- `play_replay(replay, apply) -> Result<Vec<[u8; 32]>, ReplayError>`:
  validate, then `World::new(seed)`, then per tick — every input for that
  tick in file order, `tick`, `state_hash`.
- `read_replay` / `write_replay`: pretty JSON plus trailing newline,
  parents created, byte-stable round trip. Missing file is `Io(NotFound)`;
  non-UTF-8 or bad JSON is `Malformed`.
- `play_and_verify(replay_path, golden_path, apply)`: read, validate, play,
  compare with `verify_golden` semantics. `Divergence` (boxed) carries
  replay identity plus the `Mismatch`; either file missing stays `Io`.
- `ReplayError`: `Io`, `Malformed`, `UnsupportedVersion { found }`,
  `UnorderedSchedule { index, tick, prev_tick }`,
  `OutOfRange { index, tick, total_ticks }`, `TooLarge { what, value }`,
  `ApplyFailed { tick, index, reason }`, `Divergence(Box<ReplayDivergence>)`
  — distinguishable without string matching, single-pathed `Display`.

## Dependencies

Moved from `crates/crpg-testkit/AGENTS.md` (A0-8): `crpg-sim` (path,
normal), `serde` + `serde_json` (workspace, normal — T009a replay format;
both already in the graph, no new package or version), `crpg-core` (path,
dev-only for tests), `crpg-data` + `crpg-rules` (path, dev-only for the
T016c combat adapter in `tests/` — production content loading and the typed
trace oracle; approved in-session 2026-09-26, see
`tasks/T016c.md`; both already workspace members, so the lock grows by edge
entries only). No workspace crate beyond those — later helpers add their
edges with their tasks, each justified in the task file. No external crate
beyond what the workspace already holds; anything else needs approval.

## Restored scope and machinery (A0-8)

Scope note: this file describes the T008b harness, the T009a replay, and the
T009c target policy (merged 2026-09-07; native gates passed on Windows/MSVC
and genuine WSL Ubuntu 24.04 Linux/GNU). See the
[T009c completion record](../../tasks/T009c.md).
Fixture campaigns, contract conformance suites and any further helpers arrive
with the tasks that need them; they extend this file, following the module
docs that already name their owners.

Shared test machinery, built once so it cannot fork across crates: the
hash-sequence harness and golden-file convention that T009's replay, T016's
combat and every golden file after them consume, plus the replay itself —
versioned `.replay` record/parse/validate/playback with opaque input
payloads and an exact-tick divergence report. This crate serves tests and
harnesses: its tests-only adapters never ship in a binary, while the
harness/replay library surface (`play_and_verify` and friends) is the shared
implementation `crpgc` consumes at runtime (since T009b).

## Agent log:

- 2026-09-06 (UTC) · opencode/muse-spark + T008b · Wrote the crate doc for the harness: testkit's position and one-way direction, the module map, today-vs-planned with T009/T016/E020 as named consumers, and what consumers inherit.
- 2026-09-06 (UTC) · opencode/muse-spark + testkit mismatch hardening · Recorded ADR-0010, corrected the E005 dev-dependency claim to proposed-not-fact, and noted truthful mismatch sides.
- 2026-09-06 (UTC) · opencode/gpt-5.6-sol + E005 decision · Recorded the ratified lower-layer test ownership boundary and removed the incorrect Cargo-cycle justification.
- 2026-09-06 (UTC) · opencode/gpt-5.6-sol + E020 decision · Assigned canonical-Linux enforcement to T009a under the existing workspace-test job instead of planning a placeholder CI job.
- 2026-09-06 (UTC) · opencode/muse-spark + T009a · Recorded the replay module (opaque payloads, input→tick→hash ordering, boxed divergence), the checked-in canonical-Linux fixture pair, and CI step 9 going live with T009b/T016 as the named consumers.
- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c · Aligned the architecture with Windows-primary and fully supported Linux headless hosting and independent exact-build golden scopes. Distinguished unmerged implementation from pending native verification and deferred host/extension implementation decisions to their owners.
- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c verification alignment · Replaced active pending-verification wording with the reported native gate passes and linked the completion record. Kept T009c the next review/merge priority and T009b blocked on landing, without claiming either replay task committed or merged.
- 2026-09-07 (UTC) · opencode/big-pickle + T009a/T009c merged · Updated the state and table rows to the merged replay and two-target golden gates; T009b is next.
- 2026-09-07 (UTC) · opencode/big-pickle + T009b merged · Updated the policy paragraph and the Exists/Planned table to the merged `crpgc replay` wrapper; the next replay consumer is T016.
- 2026-09-26 (UTC) · opencode/muse-spark + T016c crate opening · Recorded the tests-only combat adapter, artifact, observation, and independent native golden ownership actually built; generic harness and replay stay payload-agnostic.
- 2026-09-27 (UTC) · opencode/muse-spark + T017e crate opening · Recorded the tests-only srd adapter, extended grammar with the end op, per-attack trace shape, and independent native golden ownership before source implementation.
- 2026-09-27 (UTC) · opencode/muse-spark + T017e implementation · Aligned the module with the as-built srd replay (seed 285, K = 9, per-target goldens, combat_basic untouched).
- 2026-10-25 (UTC) · opencode-go/beam-free + A0-8 · Moved Public API, dependencies, combat/second-ruleset contract notes, and scope note from AGENTS.md into this doc; no rule meaning changed.
- 2026-10-25 (UTC) · opencode-go/beam-free + A0-8 restore · Restored trimmed text blocks verbatim (scope note, shared machinery) per A0-8 no-information-lost rule.
