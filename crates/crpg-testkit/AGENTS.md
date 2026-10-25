# crpg-testkit — agent contract

Scope: T008b harness, T009a replay, T009c target policy (merged 2026-09-07;
native gates passed). Fixtures/conformance suites arrive with their tasks.
Design/behaviour/history: `docs/architecture/crpg-testkit.md`.

## Purpose

Working contract. Design/behaviour/history:
`docs/architecture/crpg-testkit.md`. Decisions: ADR-0009 (determinism
scope), ADR-0010 (mismatch shape), E005 (one-way direction),
ADR-0012/T009c (target policy; supersedes only ADR-0009 Decision 3's
Linux-only selection). Public API list/contract: arch doc §Public API.

This crate serves tests and harnesses: tests-only adapters never ship in a
binary; the harness/replay library surface (`play_and_verify` and friends)
is the shared implementation `crpgc` consumes at runtime (since T009b).

## Invariants

1. **Fixed interleaving.** Script → tick → hash every tick from
   `World::new(seed)`; consumers do not reorder. Replay uses the same order
   with scheduled inputs: all inputs for tick `t` in file order, then `tick`,
   then `state_hash`, so index *n* means the same in both.
2. **No tolerant comparison, ever.** No fuzzy hashes, runtime platform
   golden selection, or header forgiveness beyond skipping `#` lines. Under
   ADR-0012/T009c, real `play_and_verify` selects baselines at compile time
   via `cfg(all(target_os="windows",target_arch="x86_64",target_env="msvc",debug_assertions))`
   and `cfg(all(target_os="linux",target_arch="x86_64",target_env="gnu",debug_assertions))`.
   Other targets run portable parse/play + shape checks without either
   golden. No runtime platform detection or cross-platform hash equality.
   Supported-target mismatch fails that target; absent baseline is
   `Io(NotFound)`, never a skip.
3. **Golden format append-only in compatibility.** New headers start with
   `#`; hash lines are exactly 64 lowercase hex chars. Old files verify under
   new code or the change is a migration task. Replay schema versioned the
   same way: `REPLAY_FORMAT_VERSION` is 1, unknown fields ignored on read,
   any other version is `UnsupportedVersion`, never reinterpretation.
4. **Hex and errors hand-rolled; serialization via workspace
   `serde`/`serde_json` only.** No `hex`, `thiserror`, or `tempfile`. The
   serde crates are the sole exception via existing workspace deps (already
   in graph); anything else needs approval.
5. **Tests-only adapters never ship.** Nothing in `tests/` grows a runtime
   caller; the harness/replay library surface is shared with `crpgc` at
   runtime instead. `crpgc replay` is T009b's thin consumer of
   `play_and_verify`; T016d's combat adapter owns its own decode/bind glue.
   No replay semantics in `crpg-cli`.
6. `#![forbid(unsafe_code)]`, `#![warn(missing_docs)]`; every public item
   documented (spec §15.6).
7. No clock, threads, or own randomness. File I/O only in golden functions
   and `read_replay`/`write_replay`; everything else pure. (Outside the
   determinism lint's crate list, so self-imposed.)
8. **Payloads opaque; validation before playback.** Testkit never interprets
   a payload. Every consuming entry point validates first in fixed order
   (version, metadata, counts, schedule scan), so one malformed replay gives
   one typed error. Hash-only golden yields no world-state diff:
   `ReplayDivergence` carries identity, exact tick, `Mismatch`; fabricating
   expected state is forbidden.

## Allowed dependencies

See `docs/architecture/crpg-testkit.md` §Dependencies for the full approved
edge list. No new crate or external beyond those edges without approval;
tests-only dev-edges arrive with their task, each justified in that task
file.

## Combat replay (T016c, tests-only)

See `docs/architecture/crpg-testkit.md` §Combat replay for the as-built
adapter, artifacts and golden ownership. The adapter never ships; T016d owns
the CLI glue.

## Second-ruleset replay (T017e)

See `docs/architecture/crpg-testkit.md` §Second-ruleset replay for the
as-built adapter, artifacts and golden ownership. The adapter never ships;
B5 owns the CLI proof.

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
ignored: it is the shrunk counterexample, and losing it loses the
regression.

## Known traps

- **The harness does not reach around the sim API.** Missing capability is a
  filed gap against the owning sim task, never a `pub(crate)` favour or a
  second quasi-API. The T008b commit asserts an empty sim diff; keep that
  true.
- **The fixed interleaving is not a suggestion.** A consumer hashing before
  ticking, or ticking twice per script step, produces sequences no other task
  can compare against. The failure mode is silent: everything verifies
  against itself and cross-task comparison quietly means nothing.
- **Missing golden ≠ wrong golden ≠ unreadable text.** `Io(NotFound)` and
  `Mismatch` are different variants: the first means "write one," the second
  "behaviour changed." Non-UTF-8 files are `Mismatch::InvalidUtf8`, not `Io`.
- **Never fabricate a hash side.** Length and malformed paths carry `None`
  or their own variant (ADR-0010). Zeroed `[0; 32]` sentinels made
  `expected != actual` unreliable and hid the approved hash.
- **Scope discipline belongs to CI.** Portable input
  `fixtures/replay_basic.replay`; scoped files
  `replay_basic_rust-1.98.0_x86_64-pc-windows-msvc_test-default.golden` and
  `replay_basic_rust-1.98.0_x86_64-unknown-linux-gnu_test-default.golden`.
  Rust 1.98.0, full target triple, normal test profile, default features;
  pinned toolchain and reviewed filenames enforce non-cfg scope, not runtime
  selection. Windows primary baseline; Linux server regression baseline;
  neither compared to the other (ADR-0012/T009c). Full policy:
  `docs/architecture/crpg-testkit.md` §Target-scoped replay policy.
- **E005 is a one-way ownership rule.** This crate reaches `sim`; `core`,
  `data`, `rules`, and `sim` never reach back, including through
  dev-dependencies. Cross-layer tests for those crates belong here. Cargo can
  compile some dev-only cycles, but that does not make the upward
  architecture edge acceptable. Full boundary:
  `docs/architecture/crpg-testkit.md` §Position.
- **The tests' payload shapes are not crate vocabulary.** `spawn_at`,
  `timeline`, `draw`, `despawn` in `tests/support/mod.rs` are one
  caller-side example of `ApplyInput`. Quoting them in a new feature as if
  testkit understood them is how a speculative command enum sneaks in — the
  day real intents exist, their owning task defines them.
- **Validation order is load-bearing.** Version, metadata, counts, then the
  schedule scan: reordering the checks changes which single error a malformed
  replay reports, and tests pin that precedence. Same-tick inputs are valid
  (file order kept); `tick == total_ticks` is out of range, not an empty
  trailing tick.
  At both layers: `Io(NotFound)` means "write one," `Divergence` means
  "behaviour changed" for replay and golden independently; non-UTF-8 goldens
  are `Divergence`/`InvalidUtf8`, non-UTF-8 replays are `Malformed`.
- **There is no rebless command.** Golden replacement or any scope change is
  a reviewed re-baseline event. Generate each baseline independently on its
  genuine native target through `read_replay` -> `play_replay` ->
  `write_golden`; never copy one target's sequence to bless the other.
  T009a Linux provenance may support a scope-filename rename, not
  regeneration. T009c native gates passed; provenance in
  [T009c completion record](../../tasks/T009c.md).

## Agent log

- 2026-09-06 (UTC) · opencode/muse-spark + T008b · Wrote the crate contract for the harness: API list, seven invariants, and traps for the interleaving, tolerant comparison, missing-vs-wrong goldens, scope discipline and the unratified E005 direction.
- 2026-09-06 (UTC) · opencode/muse-spark + testkit mismatch hardening · Replaced the Mismatch struct with the ADR-0010 enum (truthful sides, UTF-8 as content divergence) and recorded the no-sentinel trap.
- 2026-09-06 (UTC) · opencode/gpt-5.6-sol + E005 decision · Replaced the provisional cycle warning with the ratified one-way ownership rule and corrected its Cargo rationale.
- 2026-09-06 (UTC) · opencode/muse-spark + T009a · Extended the contract for the replay: opaque-payload API and validation invariants, the input→tick→hash ordering, boxed typed divergence reusing the ADR-0010 mismatch, the compile-time canonical-Linux gate, and traps for test-vocabulary drift, validation order, and rebless-by-command.
- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c · Replaced the active Linux-only contract with independent compile-time Windows/MSVC and Linux/GNU golden requirements. Preserved exact comparison and reviewed provenance rules without claiming pending native verification passed.
- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c verification alignment · Recorded the reported native gate passes and linked the completion record while retaining uncommitted/unmerged status. Corrected the private support-module path without changing policy or code.
- 2026-09-07 (UTC) · opencode/big-pickle + T009a/T009c merged · Updated the scope note and pending-merge wording to the merged state; T009b is next.
- 2026-09-26 (UTC) · opencode/muse-spark + T016c crate opening · Extended the contract for the tests-only combat adapter (production content loading, strict payload grammar, snapshot-based observation, independent native goldens) with the approved crpg-data/crpg-rules dev-edges before source implementation.
- 2026-09-27 (UTC) · opencode/muse-spark + A0 status reconciliation · Reconciled the never-ships wording with the as-built split (tests-only adapters never ship; the harness/replay library is shared with `crpgc` at runtime since T009b); no code, dependency, or ownership move.
- 2026-09-27 (UTC) · opencode/muse-spark + T017e crate opening · Extended the contract with the tests-only second-ruleset replay shape (srd-only adapter, accepted-only inputs with the new end op, pinned probe seed, per-target goldens) under tasks/T017e.md before source implementation.
- 2026-09-27 (UTC) · opencode/muse-spark + T017e implementation · Aligned the contract with the as-built replay (seed 285, K = 9, 13-test srd_replay suite, per-target goldens, combat_basic untouched); no commit/push/PR.
- 2026-10-25 (UTC) · opencode-go/beam-free + A0-8 · Trimmed AGENTS.md to how-to-work size by moving Public API, dependencies, combat/second-ruleset replay detail, and scope note to the architecture doc; no rule meaning changed.
