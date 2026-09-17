# crpg-cli — agent contract

Scope note: this file describes the T009b opening of the crate: the `replay`
subcommand, the provisional reference-intents apply, and the exit-code
contract — plus the T011b `validate` thin wrapper, its read-only traversal
ownership, and its gate-8 fixture gate. Later subcommands (`migrate`, `pack`,
`run`, `diff` and the T013 argument-parsing decision) extend this file with
their tasks.

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
(workspace), plus `crpg-data` (path, T011b thin edge for `validate_files`,
the path classifier, error conversion, and the canonical writer). The ALLOWED
table grants this crate everything except `crpg-godot`, so additions beyond
these are legal but still need the usual task-file justification — especially
any parser crate, which is exactly what T013 must decide. Do not add a
dependency to work around a lower-crate API gap.

## Validate contract (T011b)

`crpgc validate <campaign-root> [--json]`:

- `--json` may appear once before or after the root. Duplicate flags, extra
  positionals, unknown flags, and a missing root are usage errors (exit 2).
  A non-Unicode root parses successfully and fails later as one exit-1 `io`
  diagnostic — argument handling uses `args_os` end to end, never `args`.
- Exit `0` when validation returns no `Severity::Error` (warnings print in
  plain mode and are included in JSON mode but never fail); exit `1` on any
  `Error` — I/O, structural, or semantic — and on the two fixed-text
  internal failures (unparsable compile-time engine version, canonical
  serialization failure); exit `2` on usage. These are the same three
  buckets as replay, extended, not replaced.
- The binary collects classified bytes, calls
  `crpg_data::validate_files(files, package engine version)` once, and
  renders exactly what data returns, in returned order. Zero validation
  semantics live here.

## Validate invariants

1. **Data owns semantics; the CLI owns traversal and process behavior.**
   No kind tables, dangling-reference walks, graph traversal, diagnostic
   sorting, or second diagnostic shape may exist in `crpg-cli`. The only
   classifier is `crpg_data::campaign_document_path`; its rejections pass
   through `diagnostic_for_data_error` unchanged as `invalid_path`.
2. **Read-only sorted depth-first pre-order walk, `std` only.** List a
   directory, sort entry names by file-name bytes, visit each entry
   (symlink check via `symlink_metadata`, then classify, then read) before
   the next sibling. Never intentionally follow symlinks — not even a
   symlinked root. Logical paths are `/`-joined component text, never
   native separators. The first failure in walk order wins.
3. **One CLI-owned diagnostic shape: `io`.** Always `Error`, empty pointer,
   no fix, portable `cannot <op> <logical>: <kind>` text (`open root`,
   `list`, `classify`, `read`; stable snake_case kinds; literal
   `<campaign-root>` when no logical path exists). Never embed raw
   `io::Error` text, native separators, or absolute paths. Never
   lossy-convert a non-Unicode component.
4. **Output streams are fixed.** Plain: `Display` lines to stderr, stdout
   empty, clean runs silent. JSON: data's `canonical_json` array to stdout
   (`[]\n` when clean), stderr empty. Usage is always one `crpgc: ...`
   line on stderr with exit 2; `--json` never promises JSON for an
   unparsable command line.
5. **Gate 8 is the ordinary black-box test, never a placeholder.** The
   `gate_8_manifest_drives_every_fixture_campaign` test reads T011a's
   data-owned `expected.json` as data, requires discovered `campaign.json`
   roots to equal manifest roots exactly (non-vacuous), and invokes the
   shipped binary per entry. Future fixtures extend the manifest in
   `crpg-data`; this test picks them up generically with no CLI edit.
6. **Replay stays byte-for-byte, exit-for-exit** — plus the one authorized
   T011b repair: a flag in first position (`replay --bogus`,
   `replay --golden g`) is usage exit 2, not missing-file exit 1, with
   regression tests on both sides. No other replay semantic, default, or
   golden behavior may change here.
7. **No new dependencies for validate.** `std` plus the existing
   `crpg-data` path edge (thin data direction), `serde_json`, and the path
   crates only. The engine version type is inferred from `validate_files`,
   so no semver edge exists. No parser, walker, error, snapshot, or
   tempfile crate — T013 still owns the parser-framework decision.

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
  failure". Validate inherits the same buckets: a missing campaign root
  argument is usage (2); a supplied-but-unreadable root is `io` (1).
- **Validate tests are black-box first, seams second.** `tests/validate.rs`
  drives `env!("CARGO_BIN_EXE_crpgc")` over real fixture copies and asserts
  exit codes plus exact stream bytes. `main.rs` unit tests cover only the
  private parser matrix, the `io`/exit/render mapping with synthetic values
  (warnings-only exit 0 exists only synthetically — T011a emits Error
  today), and the `WalkFs`-injected collector seams (unreadable entries,
  symlinks, non-Unicode names, walk-order oracles). Real-symlink and
  case-collision process tests are compile-time cfg-gated where creation is
  guaranteed, never runtime-skipped.
- **No `--golden` defaulting into the fixture set.** The default golden is a
  sibling of the replay path, never a search through testkit's goldens. A
  CLI run must verify what it is told to verify.

## Agent log

- 2026-09-07 (UTC) · opencode/big-pickle + T009b · Wrote the crate contract
  for the opening task: the exit-code contract, the thin-consumer invariant,
  the verify-only line, the frozen reference apply and its drift trap, and
  the ADR-0012 cfg-gated test gate.
- 2026-09-13 (UTC) · opencode/muse-spark + T011b implementation · Extended
  the contract with the validate parser, read-only traversal ownership,
  plain/canonical output rules, the live gate-8 fixture gate, the semver
  inference dependency note, and the black-box-first test rules while
  retaining every replay invariant.