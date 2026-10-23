# crpg-cli — agent contract

Design: [`docs/architecture/crpg-cli.md`](../../docs/architecture/crpg-cli.md)
— what the crate is and how its pieces fit. This file is the working contract:
what you may do and what will break. Contracts (command grammars, exit codes,
flows) live in the architecture doc; this file keeps the rules a change must
not break.

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
   domain failure, `2` usage. They follow the established 0/1/2 convention
   (originally clap-compatible); T013 retained the hand-rolled per-command
   parsers with no library swap, so a new subcommand inherits the same
   three buckets.
4. **The reference apply is provisional and caller-owned.** Testkit owns no
   payload vocabulary (invariant 8), so the payloads mean what this crate
   says they mean — today, the miniature `spawn_at`/`timeline`/`draw`/
   `despawn` vocabulary the T009a fixture was generated against, applied
   through public `World` APIs only. It must stay byte-identical to
   `crpg-testkit/tests/support/mod.rs::test_apply` while the fixture is the
   reference: "byte-identical" covers apply semantics, so the binary and
   testkit verify the same fixture to the same result. The CLI's diagnostic
   wording is its own — it says `unknown intent payload` where testkit says
   `unknown test payload` (pinned by T016d). When T014/T016 define real intents, this apply is
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

Per-subcommand invariants (validate, migrate, T013, combat replay) moved to
`docs/architecture/crpg-cli.md` (§Validate invariants, §Migrate invariants,
§T013 invariants, §Combat replay invariants) — keep their wording there.

## Allowed dependencies

`crpg-testkit` (path), `crpg-sim` (path), `crpg-core` (path), `serde_json`
(workspace), plus `crpg-data` (path, T011b thin edge for `validate_files`,
the path classifier, error conversion, and the canonical writer), and
`same-file` (T013 review hardening for cross-platform mutating-command file
identity). The ALLOWED
table grants this crate everything except `crpg-godot`, so additions beyond
these are legal but still need the usual task-file justification — especially
any parser crate, which T013 decided against by retaining hand-rolled
per-command parsers with no new dependency. Do not add a
dependency to work around a lower-crate API gap.

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
- **No `--golden` defaulting into the fixture set.** The default golden is a
  sibling of the replay path, never a search through testkit's goldens. A
  CLI run must verify what it is told to verify.
- **A supplied root may not traverse a symlinked ancestor.** The collector
  checks lexical ancestors before touching the terminal root, matching the
  lock path discipline. This applies to validate, migrate, explain and fmt;
  accepting `alias/campaign` where `alias` is a symlink would make later
  read or rewrite guarantees meaningless.

Command-specific traps (validate/migrate/T013/lock/LLM trials) moved to
`docs/architecture/crpg-cli.md` §Known traps (moved from AGENTS.md).

## Agent log

- 2026-09-26 (UTC) · opencode/muse-spark + T016d documentation · Extended the scope note and added the explicit two-mode combat replay contract (parser, loading, adapter ownership, precedence, and black-box-first test rules) before source changes, superseding the reference-only statements while retaining legacy semantics and the testkit boundary.

- 2026-09-07 (UTC) · opencode/big-pickle + T009b · Wrote the crate contract
  for the opening task: the exit-code contract, the thin-consumer invariant,
  the verify-only line, the frozen reference apply and its drift trap, and
  the ADR-0012 cfg-gated test gate.
- 2026-09-13 (UTC) · opencode/muse-spark + T011b implementation · Extended
  the contract with the validate parser, read-only traversal ownership,
  plain/canonical output rules, the live gate-8 fixture gate, the semver
  inference dependency note, and the black-box-first test rules while
  retaining every replay invariant.
- 2026-09-18 (UTC) · opencode/muse-spark + T012b implementation · Extended
  the contract with the migrate parser/process contract, the reused
  collector plus `load_campaign`/`serialize_campaign` calls, the
  `RewriteFs` writer/error seams with bounded partial-write limits, the
  black-box golden plus seam test rules, and the no-lower-crate-edit rule
  while retaining every replay/validate invariant.
- 2026-09-19 (UTC) · opencode/muse-spark + T013 implementation · Extended
  the contract with the six T013 command grammars, the retained hand-rolled
  per-command parser and exit rules, the exact data/harness API usage with
  ownership traps, the `create_new` lock seam, the five-suite plus
  literal-trial acceptance rules, and the dependency stop rule while
  retaining every replay/validate/migrate invariant.
- 2026-09-19 (UTC) · opencode/muse-spark + T013 review remediation (R7) · Aligned stale parser-swap/decision wording with the landed T013 choice: retained hand-rolled `args_os` per-command parsers, the established 0/1/2 convention with no future swap, and no parser-crate dependency.
- 2026-09-22 (UTC) · opencode/gpt-5.6-sol + T013 independent review hardening · Recorded root-ancestor rejection, normalized lock input/output alias protection, the stable-std hard-link limitation, and byte-pinned dual-set LLM reruns after adversarial review exposed those missing assumptions.
- 2026-09-22 (UTC) · opencode/gpt-5.6-sol + T013 hard-link authorization · Added the maintainer-authorized `same-file` exception so lock rejects hard-link aliases of every input cross-platform instead of accepting the stable-std limitation identified by review.
- 2026-09-22 (UTC) · opencode/gpt-5.6-sol + T013 fmt alias hardening · Extended the authorized filesystem-identity check to fmt's complete collected regular-file set, preventing a recognized rewrite target from mutating ignored hard-linked content.
- 2026-09-22 (UTC) · opencode/gpt-5.6-sol + T013 shared-writer alias hardening · Applied the same complete regular-file identity preflight to migrate, preserving its existing ignored-file guarantee through hard-link aliases as well as ordinary paths.
- 2026-09-27 (UTC) · opencode/muse-spark + T017f crate opening · Extended the contract with the CLI-only second-ruleset proof shape (single end-op decode with error-string parity, unchanged-command black-box suite over both campaigns) under tasks/T017f.md before source implementation.
- 2026-09-27 (UTC) · opencode/muse-spark + T017f implementation · Aligned the contract with the as-built proof (end decode with result-shape guard, 10-test srd_replay suite, B4 artifacts read-only); no commit/push/PR.
- 2026-10-16 (UTC) · opencode-go/beam-free + A0-8 · Trimmed AGENTS.md to how-to-work by moving detailed contracts/invariants/traps into the architecture doc, leaving crate-wide rules, gates, and cross-cutting traps here.
- 2026-10-21 (UTC) · opencode-go/muse-spark-1.3-contributor + S12-docs · Clarified invariant 4's byte-identical scope (apply semantics, not diagnostic wording) and pinned the CLI's own `unknown intent payload` wording per D29k; nothing else changed.
"""