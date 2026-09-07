# crpg-testkit — agent contract

Scope note: this file describes the T008b harness, the T009a replay, and the
T009c target policy (merged 2026-09-07; native gates passed on Windows/MSVC
and genuine WSL Ubuntu 24.04 Linux/GNU). See the
[T009c completion record](../../tasks/T009c.md).
Fixture campaigns, contract conformance suites and any further helpers arrive
with the tasks that need them; they extend this file, following the module
docs that already name their owners.

## Purpose

Design doc: [`docs/architecture/crpg-testkit.md`](../../docs/architecture/crpg-testkit.md)
— what the crate is and how its pieces fit. This file is the working contract:
what you may do and what will break. The dependency direction is ratified by
E005; the determinism scope it operates under is ADR-0009; the mismatch shape
is ADR-0010. ADR-0012/T009c supersedes only ADR-0009 Decision 3's Linux-only
selection, not its exact-build scope or hash-exclusion governance.

Shared test machinery, built once so it cannot fork across crates: the
hash-sequence harness and golden-file convention that T009's replay, T016's
combat and every golden file after them consume, plus the replay itself —
versioned `.replay` record/parse/validate/playback with opaque input
payloads and an exact-tick divergence report. This crate serves tests and
harnesses; it never ships in a binary.

## Public API  (changing this requires an ADR)

`run_hash_sequence`, `ScriptStep`, `write_golden`, `verify_golden`,
`Mismatch`, `HarnessError`, and the replay surface: `Replay`,
`ReplayInput`, `ApplyInput`, `ReplayDivergence`, `ReplayError`,
`REPLAY_FORMAT_VERSION`, `MAX_REPLAY_TICKS`, `MAX_REPLAY_INPUTS`,
`Replay::new`, `validate_replay`, `play_replay`, `write_replay`,
`read_replay`, `play_and_verify`.

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

## Invariants

1. **The interleaving is fixed.** Script → tick → hash, in that order, for
   every consumer. It is what makes sequences comparable across tasks.
   Consumers do not reorder it. Replay playback is the same interleaving
   with scheduled inputs in place of the script step: every input for tick
   `t` in file order, then `tick`, then `state_hash` — so a replay hash at
   index *n* means what a harness hash at index *n* means.
2. **No tolerant comparison, ever.** No fuzzy hashes, no per-platform
   goldens chosen at runtime, no header-aware forgiveness beyond skipping
   `#` lines. Under ADR-0012/T009c, real `play_and_verify` comparisons select
   independent baselines at compile time with
   `cfg(all(target_os = "windows", target_arch = "x86_64", target_env = "msvc", debug_assertions))`
   and
   `cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu", debug_assertions))`.
   Everywhere else runs portable parse/play repeatability and shape checks
   without reading either scoped golden. Never use runtime platform detection
   or assert cross-platform hash equality. A supported-target mismatch fails
   that target; an absent baseline fails with `Io(NotFound)`, never a skip.
3. **Golden format is append-only in compatibility.** New header lines must
   start with `#`; hash lines stay exactly 64 lowercase hex chars. Old files
   verify under new code or the format change is a migration with its own
   task. The replay schema is versioned the same way: `REPLAY_FORMAT_VERSION`
   is 1, unknown fields are ignored on read, and any other version fails as
   `UnsupportedVersion` — never a silent reinterpretation.
4. **Hex and errors stay hand-rolled; serialization goes through the
   workspace `serde`/`serde_json` only.** No `hex` crate, no `thiserror`, no
   `tempfile` — none of them is worth a node in the dependency graph for
   what a dozen lines do. `serde` + `serde_json` are the one exception, and
   only via the existing workspace dependencies: both were already in the
   graph (sim serializes through them), so replay adds no new package or
   version. Anything beyond that needs approval.
5. **This crate never ships.** Nothing here may grow a runtime-only caller;
   anything a binary needs is a different crate's API. `crpgc replay` is
   T009b's thin consumer of `play_and_verify` — no replay semantics live in
   `crpg-cli`.
6. `#![forbid(unsafe_code)]`, `#![warn(missing_docs)]`. Every public item
   has a doc comment (spec §15.6).
7. No clock, no threads, no randomness of its own. File I/O is confined to
   the golden functions and `read_replay`/`write_replay`; everything else
   is pure. (This crate is outside
   the determinism lint's crate list, which makes the discipline
   self-imposed — hold it anyway; the day testkit output feeds a golden,
   sloppiness here becomes flakiness everywhere.)
8. **Payloads are opaque; validation runs before playback.** Testkit never
   interprets a payload — no combat, networking, or campaign command enum
   enters this crate until a task with real intents owns it. Every entry
   point that consumes a replay validates first, in fixed check order
   (version, metadata, counts, schedule scan), so one malformed replay
   always reports one typed error. A hash-only golden yields no
   world-state diff: `ReplayDivergence` carries identity, exact tick, and
   the `Mismatch` — fabricating expected state is forbidden.

## Allowed dependencies

`crpg-sim` (path, normal), `serde` + `serde_json` (workspace, normal —
T009a replay format; both already in the graph, no new package or version),
`crpg-core` (path, dev-only for tests).
No workspace crate beyond those two — later helpers (data fixtures, rules
suites) add their edges with their tasks, each justified in the task file.
No external crate beyond what the workspace already holds; anything else
needs approval.

## Definition of done for any change

```
cargo fmt --all
cargo clippy -p crpg-testkit --all-targets -- -D warnings
cargo test -p crpg-testkit
python tools/lint/deps.py
python tools/lint/determinism.py
python -m unittest discover -s tools/lint -p "test_*.py"
```

Any `proptest-regressions/` file a failure produces is **committed**, not
ignored: it is the shrunk counterexample, and losing it loses the regression.

## Known traps

- **The harness does not reach around the sim API.** Missing capability is
  a filed gap against the owning sim task, never a `pub(crate)` favour or a
  second quasi-API. The T008b commit asserts an empty sim diff; keep that
  true.
- **The fixed interleaving is not a suggestion.** A consumer hashing before
  ticking, or ticking twice per script step, produces sequences no other
  task can compare against. The failure mode is silent: everything verifies
  against itself and cross-task comparison quietly means nothing.
- **Missing golden ≠ wrong golden ≠ unreadable text.** `Io(NotFound)` and
  `Mismatch` are different variants on purpose. Do not merge them into "no
  approved baseline" — the first means "write one," the second means
  "behaviour changed," and confusing them wastes exactly the investigation
  the harness exists to shorten. Non-UTF-8 files are `Mismatch::InvalidUtf8`,
  not `Io`.
- **Never fabricate a hash side.** Length and malformed paths carry `None`
  or their own variant (ADR-0010). Zeroed `[0; 32]` sentinels made
  `expected != actual` unreliable and hid the approved hash.
- **Scope discipline belongs to CI.** The portable input is
  `fixtures/replay_basic.replay`. The scoped files under `goldens/` are
  `replay_basic_rust-1.98.0_x86_64-pc-windows-msvc_test-default.golden` and
  `replay_basic_rust-1.98.0_x86_64-unknown-linux-gnu_test-default.golden`.
  Rust 1.98.0, the complete target triple, normal test profile, and default
  features define each scope. The pinned toolchain and reviewed filenames
  enforce the non-cfg scope elements, not runtime selection. Windows owns the
  primary behavioural baseline; Linux owns a supported server regression
  baseline. Neither is compared to the other (ADR-0012/T009c).
- **E005 is a one-way ownership rule.** This crate reaches `sim`; `core`,
  `data`, `rules`, and `sim` never reach back, including through
  dev-dependencies. Cross-layer tests for those crates belong here. Cargo can
  compile some dev-only cycles, but that does not make the upward architecture
  edge acceptable.
- **The tests' payload shapes are not crate vocabulary.** `spawn_at`,
  `timeline`, `draw`, `despawn` in `tests/support/mod.rs` are one caller-side
  example of `ApplyInput`. Quoting them in a new feature as if testkit
  understood them is how a speculative command enum sneaks in — the day real
  intents exist, their owning task defines them.
- **Validation order is load-bearing.** Version, metadata, counts, then the
  schedule scan: reordering the checks changes which single error a malformed
  replay reports, and tests pin that precedence. Same-tick inputs are valid
  (file order kept); `tick == total_ticks` is out of range, not an empty
  trailing tick.
- **Missing golden ≠ wrong golden, at both layers.** `Io(NotFound)` means
  "write one," `Divergence` means "behaviour changed" — for the replay file
  and the golden file independently. Non-UTF-8 goldens are `Divergence`
  carrying `InvalidUtf8` (content divergence, ADR-0010), while non-UTF-8
  replays are `Malformed` (rejected input, never played).
- **There is no rebless command.** Golden replacement or any scope change is
  a reviewed re-baseline event. Generate each baseline independently on its
  genuine native target through `read_replay` -> `play_replay` ->
  `write_golden`; never copy one target's sequence to bless the other.
  T009a's genuine Linux provenance may support a scope-filename rename, not a
  regeneration claim. T009c native Windows/MSVC and genuine WSL Ubuntu
  Linux/GNU required gates passed; provenance and results belong to the
  [T009c completion record](../../tasks/T009c.md).

## Agent log

- 2026-09-06 (UTC) · opencode/muse-spark + T008b · Wrote the crate contract for the harness: API list, seven invariants, and traps for the interleaving, tolerant comparison, missing-vs-wrong goldens, scope discipline and the unratified E005 direction.
- 2026-09-06 (UTC) · opencode/muse-spark + testkit mismatch hardening · Replaced the Mismatch struct with the ADR-0010 enum (truthful sides, UTF-8 as content divergence) and recorded the no-sentinel trap.
- 2026-09-06 (UTC) · opencode/gpt-5.6-sol + E005 decision · Replaced the provisional cycle warning with the ratified one-way ownership rule and corrected its Cargo rationale.
- 2026-09-06 (UTC) · opencode/muse-spark + T009a · Extended the contract for the replay: opaque-payload API and validation invariants, the input→tick→hash ordering, boxed typed divergence reusing the ADR-0010 mismatch, the compile-time canonical-Linux gate, and traps for test-vocabulary drift, validation order, and rebless-by-command.
- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c · Replaced the active Linux-only contract with independent compile-time Windows/MSVC and Linux/GNU golden requirements. Preserved exact comparison and reviewed provenance rules without claiming pending native verification passed.
- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c verification alignment · Recorded the reported native gate passes and linked the completion record while retaining uncommitted/unmerged status. Corrected the private support-module path without changing policy or code.
- 2026-09-07 (UTC) · opencode/big-pickle + T009a/T009c merged · Updated the scope note and pending-merge wording to the merged state; T009b is next.
