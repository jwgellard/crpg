# crpg-cli — agent contract

Scope note: this file describes the T009b opening of the crate: the `replay`
subcommand, the provisional reference-intents apply, and the exit-code
contract. Later subcommands (`validate`, `migrate`, `pack`, `run`, `diff`
and the T013 argument-parsing decision) extend this file with their tasks.

## Purpose

Design doc: [`docs/architecture/crpg-cli.md`](../../docs/architecture/crpg-cli.md)
— what the crate is and how its pieces fit. This file is the working contract:
what you may do and what will break. The crate is the human-and-CI face of
the simulation core: `crpgc` maps command lines to crate APIs and maps
results to exit codes. Behavioural authority stays in the lower crates; the
CLI may call them in new combinations, never reimplement their semantics.

## Public contract

`crpgc replay <replay-path> [--golden <golden-path>]`:

- `--golden` defaults to the replay path with its extension replaced by
  `.golden` (`foo.replay` -> `foo.golden`). Omitting it is a path decision,
  never a "use any golden" trap.
- Exit `0` on sequence equality (nothing printed), `1` on any
  `crpg_testkit::ReplayError` (single-pathed `Display` to stderr), `2` on a
  usage error. These codes are a public CI-facing contract, stable from the
  first version.
- The binary calls `crpg_testkit::play_and_verify` and maps the result. It
  does not read JSON, interleave, or construct a divergence itself.

## Invariants

1. **Thin consumer.** No replay semantics live in `crpg-cli`:
   `read_replay`/validation/interleaving/comparison all live in testkit
   (`play_and_verify`). A change to replay semantics happens in testkit and
   the CLI picks it up unchanged. Replay logic that would have to live here
   proves testkit's API is incomplete — file it there, do not reach around.
2. **Verify-only.** There is no `--write`, rebless, or recording mode.
   Golden generation is a reviewed re-baseline event per native target in
   testkit (ADR-0012). This line exists so `crpgc replay --write` cannot
   become an accidental bless-by-command.
3. **Exit codes never mean different things.** `0` verified, `1` replay
   domain failure, `2` usage. They follow clap's convention so the T013
   parser swap stays script-compatible; a new subcommand inherits the same
   three buckets.
4. **The reference apply is provisional and caller-owned.** Testkit owns no
   payload vocabulary (invariant 8), so the payloads mean what this crate
   says they mean — today, the miniature `spawn_at`/`timeline`/`draw`/
   `despawn` vocabulary the T009a fixture was generated against, applied
   through public `World` APIs only. It must stay byte-identical* to
   `crpg-testkit/tests/support/mod.rs::test_apply` while the fixture is the
   reference, or the binary and testkit would verify the same fixture to
   different results. When T014/T016 define real intents, this apply is
   replaced by theirs; nothing outside this crate may import it.
5. `#![forbid(unsafe_code)]`. No OS-specific process/filesystem logic below
   the crate boundaries it sits on. No engine dependency — `crpg-godot` is
   the one crate this crate may not import, structurally (the deps
   ALLOWED-table exclusion), not by convention.
6. **Determinism discipline is self-imposed here.** The closure-style apply
   and reviewable goldens keep it honest.
7. **Target scope is ADR-0012 exact-build.** The CLI tests select the native
   golden at compile time with the same two cfg guards testkit uses; they
   never detect the platform at runtime and never skip the scoped check into
   a false green. Portable cases (divergence, malformed, missing, usage) run
   everywhere.

## Allowed dependencies

`crpg-testkit` (path), `crpg-sim` (path), `crpg-core` (path), `serde_json`
(workspace). The ALLOWED table grants this crate everything except
`crpg-godot`, so additions beyond these are legal but still need the usual
task-file justification — especially any parser crate, which is exactly what
T013 must decide. Do not add a dependency to work around a lower-crate API
gap.

## Definition of done for any change

```
cargo fmt --all
cargo clippy -p crpg-cli --all-targets -- -D warnings
cargo test -p crpg-cli
cargo test --workspace
python tools/lint/deps.py
python tools/lint/determinism.py
python -m unittest discover -s tools/lint -p "test_*.py"
```

Success and default-golden tests are cfg-gated to the ADR-0012 supported
targets (Windows/MSVC and Linux/GNU) and must pass on both — a failure on
either is a defect, not a best-effort gap.

## Known traps

- **The binary AS a black box is the contract.** Integration tests drive
  `env!("CARGO_BIN_EXE_crpgc")` and assert exit codes plus single-pathed
  stderr diagnostics — never replay scene internals. Do not add a library
  facade for the binary so tests can "call it in-process"; the exit-code
  contract exists precisely to keep it a process.
- **Missing baseline and wrong baseline are different exit paths.**
  `Io(NotFound)` is exit 1 with an I/O diagnostic; divergence is exit 1 with
  a "diverged at tick N" diagnostic. A test asserting "exit 1" is the useful
  contract; a test string-matching the reverse is a trap.
- **The reference apply must not drift from the fixture's generator.**
  `as_f64() as f32` casts, slot indexing, replace-on-insert timeline
  semantics — the CLI apply and testkit's `test_apply` are two frozen copies
  of one semantics while the fixture lives; a "cleanup" that differs is a
  silent behavioural fork. The day real intents exist, drop this apply for
  theirs; do not half-migrate it.
- **Exit code 2 is usage, not file trouble.** Unknown subcommand, missing
  replay path, unknown flag, missing `--golden` value. Keep filesystem and
  content failures in 1 so scripts can distinguish "user error" from "CI
  failure".
- **No `--golden` defaulting into the fixture set.** The default golden is a
  sibling of the replay path, never a search through testkit's goldens. A
  CLI run must verify what it is told to verify.

## Agent log

- 2026-09-07 (UTC) · opencode/big-pickle + T009b · Wrote the crate contract
  for the opening task: the exit-code contract, the thin-consumer invariant,
  the verify-only line, the frozen reference apply and its drift trap, and
  the ADR-0012 cfg-gated test gate.