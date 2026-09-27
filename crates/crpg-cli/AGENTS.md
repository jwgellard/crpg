# crpg-cli — agent contract

Scope note: this file describes the T009b opening of the crate: the `replay`
subcommand, the provisional reference-intents apply, and the exit-code
contract — plus the T011b `validate` thin wrapper, its read-only traversal
ownership, and its gate-8 fixture gate — plus the T012b `migrate`
explicit-save wrapper, its preflight/rewrite boundary, and its data-golden
gate — plus the six T013 thin wrappers (`new`, `schema`, `explain`, `fmt`,
`lock`, `run`), their retained hand-rolled per-command parser decision, and
their black-box plus literal-LLM-trial acceptance — plus the T016d combat
replay mode (`--campaign`, private combat adapter, black-box acceptance).
Later subcommands (`pack`, `diff`) extend this file with their tasks.

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
   domain failure, `2` usage. They follow the established 0/1/2 convention
   (originally clap-compatible); T013 retained the hand-rolled per-command
   parsers with no library swap, so a new subcommand inherits the same
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
the path classifier, error conversion, and the canonical writer), and
`same-file` (T013 review hardening for cross-platform mutating-command file
identity). The ALLOWED
table grants this crate everything except `crpg-godot`, so additions beyond
these are legal but still need the usual task-file justification — especially
any parser crate, which T013 decided against by retaining hand-rolled
per-command parsers with no new dependency. Do not add a
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
   tempfile crate — T013 retained hand-rolled parsing crate-wide with no
   library.

## Migrate contract (T012b)

`crpgc migrate <campaign-root>`:

- Exactly one root and no flags: a missing argument, an extra positional, or
  any flag (including `--json`, `--check`, `--dry-run`, `--to` and `--golden`)
  is usage exit 2 with one `crpgc: ...` line. A non-Unicode root parses and
  fails later as exit-1 `io` — `args_os` end to end, never `args`.
- Exit `0` silent (both streams empty, including a second no-op run) when all
  required canonical replacements completed or no change was needed; exit `1`
  on collection, structural/migration, serialization, or write failure with
  one data-owned `Display` line or one CLI-owned `io` line on stderr; exit
  `2` on usage. Stdout is always empty.
- The binary reuses the T011b read-only collector verbatim, calls
  `crpg_data::load_campaign(files, package engine version)` once and
  `crpg_data::serialize_campaign(loaded)` once, compares bytes, and replaces
  differing recognized files in lexical `SourcePath` order. No
  `validate_files`, no individual migration steps, no schema-tag, digest, or
  semantic logic lives here.

## Migrate invariants

1. **Data owns version chains; the CLI owns the explicit save.** No
   path-family table, version parser, tag mutation, JSON decode, digest
   recomputation, or semantic check may exist in `crpg-cli`. The only
   classifier is `crpg_data::campaign_document_path`; T012a alone owns chain
   semantics and their insertion into T010's fixed read order.
2. **Preflight first, zero writes on failure.** All collection, load, and
   serialization work succeeds for the entire map before any write starts. A
   key-set mismatch is `crpgc migrate: internal document set failure` before
   writing; an unparsable compile-time engine version is `crpgc migrate:
   internal engine version failure`. Serialization `DataError` converts
   through `diagnostic_for_data_error` unchanged.
3. **Bounded rewrite, no atomicity promise.** Per differing file in lexical
   order: recheck root/ancestor/target metadata without following symlinks,
   re-read and require equality with the collected original (`source_changed`
   on drift, `not_a_file` for a directory/other target), open the existing
   file with truncation, `write_all`, `sync_all`; stop at the first failure.
   Earlier files may stay replaced and the failing file partially written —
   never claim rollback, atomic replacement, crash recovery, or a directory
   transaction. `check`/`open`/`write`/`sync` with stable `cannot <op>
   <logical>: <kind>` text are the only CLI-owned diagnostics; ancestor
   failures name the target logical path.
4. **No migrate-semantic dependencies.** The engine version type is inferred
   from `load_campaign`, so no semver edge exists. The only post-T012b
   exception is T013 review's maintainer-authorized `same-file` check for the
   shared writer's cross-platform hard-link safety. No clap, walker, tempfile,
   error, broader platform, or atomic-write crate; parsing stays hand-rolled
   crate-wide.
5. **No lower-crate edits.** Consume committed T012a data APIs, schemas, and
   fixtures without editing them. If the public API or fixture contract is
   insufficient, stop and report the data gap; do not patch data from this
   crate.

## T013 contracts (new / schema / explain / fmt / lock / run)

```
crpgc new <type> --slug <s> --id <id> [--entry-id <id>]
crpgc schema <type>
crpgc explain <id> [--root <campaign-root>]
crpgc fmt [<campaign-root>] [--check]
crpgc lock [<campaign-root>] --catalog <catalog-path>
crpgc run --ticks N --hash-every M [--seed S]
```

- `explain`, `fmt` and `lock` default their root to `.`. No ancestor
  search, environment override, implicit stdin, or repository-root lookup.
  Relative paths, including the catalog, are relative to the process
  working directory. Existing `replay`/`validate`/`migrate` contracts stay
  exactly as documented above.
- The hand-rolled `args_os` parser is retained, organized into private
  per-command parsers (`parse_new`, `parse_schema`, `parse_explain`,
  `parse_fmt`, `parse_lock`, `parse_run`). Each named option occurs at most
  once before or after positionals; values are separate tokens, never
  `--key=value`. Unknown flags, duplicates, missing values, extra
  positionals, `--`, short flags, and unlisted `--help`/`--version` forms
  are exit 2 with exactly `crpgc: usage: <syntax>\n` for the recognized
  command — one line for every usage error of that command, never echoing
  input. Text arguments (type/slug/id/numbers) must be Unicode; path
  arguments stay `OsString` until I/O validation, so a non-Unicode path
  parses and fails later as exit-1 `io`. Parsing completes before any
  filesystem access or output.
- `new`: type is `creature`/`item`/`dialogue`/`quest`; `--slug`/`--id`
  required, `--entry-id` required only for dialogue/quest and forbidden
  otherwise, with distinct object/entry ids. Slug grammar
  `[a-z0-9]+(?:-[a-z0-9]+)*` is checked without a regex crate; ids parse via
  `Ulid::from_str` with no trimming (core aliases apply, all-zero is
  valid). Exit `0` writes one canonical typed `write_document` through
  stdout; exit `1` is a serialization failure as one data-owned line;
  exit `2` is bad grammar, unsupported type, malformed id, equal ids, or
  invalid slug. No clock, RNG, registry, or filesystem lookup; notes
  absent; repeated operands repeat bytes.
- `schema`: exactly one of the seventeen stems (`campaign`, `world`,
  `area`, `creature`, `item`, `dialogue`, `quest`, `faction`, `graph`,
  `placements`, `triggers`, `locale`, `variables`, `campaign-lock`,
  `assets-lock`, `placement`, `action-signature`) mapping to
  `<stem>.schema.json` in `generated_schemas()`. Exit `0` writes the
  returned bytes unchanged; exit `1` is a generation failure as one
  data-owned line; exit `2` is bad grammar or unknown stem.
- `explain`: exactly one id plus optional `--root`. Flow is parse id ->
  collect -> `load_campaign(files, package engine version)` once ->
  `explain_object(&campaign, id)` once -> emit bytes, consuming the exact
  landed T013a signature with no CLI walker, kind table, or semantic
  prerequisite. Exit `0` writes `Some(bytes)` untouched; exit `1` is a
  collection/load/query error as one data-owned line, or `None` as exactly
  `crpgc explain: object not found: <canonical-uppercase-id>\n`;
  exit `2` is bad grammar or malformed id text (malformed beats a missing
  root; structural failure wins over absence).
- `fmt`: zero or one root plus optional `--check`. Reuses migrate's
  collect -> load -> serialize -> key-set check -> byte-diff plan; default
  mode saves all differing recognized files with the T012b writer and its
  exact preflight/no-op/recheck/error/partial-write contract (key-set
  mismatch is `crpgc fmt: internal document set failure`). `--check` is
  the same preflight without opening files for writing: differences are
  exit `1` with `crpgc fmt: noncanonical: <logical>\n` per file in lexical
  `SourcePath` order. Silent exit `0` otherwise. Semantic findings never
  block formatting.
- `lock`: zero or one root plus exactly one `--catalog` path (omitted
  catalog is usage). Reads `campaign.json`, `assets/assets.lock`, and the
  catalog in that order without following observed symlinks, requiring
  regular files and Unicode components with the CLI I/O checks
  (`<catalog>` is the stable logical label for the catalog). Campaign
  decodes via `read_document` requiring `Document::Campaign` (otherwise
  exactly `crpgc lock: expected campaign document\n`); assets via
  `read_assets_lock`; the catalog decodes directly with
  `serde_json::from_slice::<Vec<PackageCandidate>>`, never through
  `Value` (syntax or typed failures are exactly
  `crpgc lock: invalid catalog\n`). Then `make_campaign_lock` (no separate
  resolve/digest reproduction) and `write_campaign_lock` (note is the data
  constructor's `None`). All input/resolve/serialize work completes before
  the output is touched. Existing regular output reuses the T012b
  discipline with raw-byte no-op equality; missing output rechecks
  ancestors and uses `create_new` (a concurrently appearing output is
  `source_changed`); directories are never created, symlinks never
  followed, non-regular outputs never replaced. Save failures use
  `check`/`open`/`write`/`sync` on `campaign.lock` with no rollback claim.
  `load_campaign` is never called.
- `run`: required `--ticks N` and `--hash-every M` plus optional
  `--seed S` defaulting to 0. Numbers are ASCII decimal digits only
  (`0 <= N <= 1000000`, `1 <= M <= 1000000`, `S` any `u64`; leading zeroes
  accepted; signs, whitespace, hex, separators, and overflow are usage).
  Calls `crpg_testkit::run_hash_sequence(seed, ticks, Box::new(|_| {}))`
  once — the empty world's no-op script — and prints samples after
  completed ticks `M, 2M, ... <= N` as `<k> <64-lowercase-hex>\n` from
  index `k-1`. No header, initial-state, or forced final partial sample.
  Exit `1` is a stream failure only.

## T013 invariants

1. **Data owns behaviour; the CLI owns process and I/O.** No second
   canonicalizer, version decision, reference walker, resolver, digest
   recomputation, tick/hash loop, or semantic check may exist in
   `crpg-cli`. Shared ownership errors convert through
   `diagnostic_for_data_error` with unchanged `Display` lines; CLI-owned
   I/O keeps the portable `cannot <op> <logical>: <kind>` shape with no
   raw OS text, absolute paths, native separators, or debug dumps.
2. **One reviewed filesystem exception.** Existing CLI edges to
   core/data/sim/testkit and serde_json, plus std and the maintainer-authorized
   `same-file` identity edge; semver types inferred. No clap, RNG, ULID
   generator, walker, tempfile, schema validator, error, or hashing
   dependency. Manifests, Cargo.lock, deny policy, and ALLOWED stay
   unchanged.
3. **One implementation crate.** No lower-crate source/test/doc edits,
   testkit changes, contracts, toolchain, root manifest, workflow, lint,
   or status-file changes. If a public API cannot support the contract,
   stop and report the owning-crate gap; never duplicate its semantics or
   ship a successful placeholder.
4. **Tests are black-box first, seams second.** The five new suites drive
   `env!("CARGO_BIN_EXE_crpgc")` over private temp copies with paths from
   `CARGO_MANIFEST_DIR`; `main.rs` unit tests cover only parser matrices
   and injected I/O/stream/internal failures. Existing data fixtures,
   schemas, snapshots, and target goldens are read-only. Never bless
   outputs, regenerate baselines, weaken tests, or runtime-skip
   permissions, symlinks, fixtures, or goldens into success.

## Combat replay (T016d)

`crpgc replay <replay-path> [--golden <golden-path>] [--campaign <campaign-root>]`:

- The replay path stays the first argument after `replay`; flags before that
  path remain usage errors. After the path, `--golden` and `--campaign` may
  appear in either order, at most once each, with separate-token values.
  `Command::Replay` gains `campaign_root: Option<PathBuf>`; the hand-written
  `args_os` parser and process rendering conventions are retained.
- `--campaign` explicitly selects combat payload semantics and supplies the
  campaign tree; its absence selects the existing reference adapter unchanged.
  No content sniffing, replay-name special case, implicit campaign search,
  auto-fallback, or mixed vocabulary. A combat replay without the flag reaches
  the reference adapter's ordinary unknown-intent error; a reference replay
  with the flag fails combat payload version decoding. Golden default remains
  `replay_path.with_extension("golden")`; no target detection, fixture search,
  or rebless operation.
- All paths remain `OsString`/`PathBuf`; relative paths resolve against the
  process working directory independently (campaign is not relative to the
  replay). The campaign may live outside the repository under a renamed
  directory. Existing `--golden` value parsing is unchanged, including its
  treatment of flag-looking value tokens. A missing `--campaign` value or a
  next token recognized by `flag_text` as a flag is exactly
  `crpgc: --campaign needs a value\n` (exit 2), detected before the duplicate
  check; a second `--campaign` is exactly
  `crpgc: --campaign given more than once\n` (exit 2). Unknown flags, extra
  positionals, `--campaign=...`, `--`, recording flags, and other unlisted
  syntax remain usage errors. The whole command parses before any I/O; a
  non-Unicode campaign root parses and reaches the collector's exit-1
  diagnostic rather than being lossy-converted or rejected as usage. The
  documented usage text now includes `--campaign`.
- Legacy mode retains its single `play_and_verify` call with
  `apply::reference_intents()` and the existing Outcome mapping. Combat mode,
  in order: collect via `collect_campaign_files_with`/`RealFs` (sorted
  traversal, classifier, ignored-file, ancestor/symlink, non-Unicode, portable
  I/O diagnostics reused verbatim); `load_for_command` with
  `env!("CARGO_PKG_VERSION")` calling `load_campaign` once (defensive version
  failure is exactly `crpgc replay: internal engine version failure\n`, exit
  1; structural/load errors use `data_outcome`; collector errors use
  `io_outcome`); `crpg_data::validate(&loaded)` once requiring an empty
  finding list (each finding prints with the existing diagnostic-line
  formatter in returned order, stdout empty, exit 1 — strict, so future
  warnings also fail here without changing `validate`'s warnings-only policy;
  never sorted, reconstructed, or duplicated); move the loaded campaign into
  the private combat adapter and call `play_and_verify` once (success is exit
  0 with both streams empty; replay/apply/divergence errors use
  `Outcome::cli_error(CliError::Replay(...))`). Error precedence is usage →
  campaign collection → engine/load → campaign semantic findings → replay
  read/validation → application → golden read/comparison. Replay metadata stays
  descriptive (no `campaign_id`-to-path equation, slug comparison,
  engine/version enforcement, or silent campaign selection); the supplied
  campaign already contains its ruleset, ability, and table documents.
- The private combat adapter lives in `mod combat_apply` with
  `pub(crate) fn combat_intents(loaded: crpg_data::LoadedCampaign) ->
  crpg_testkit::ApplyInput`: an owned `move` closure over the loaded
  documents, an initially empty `BTreeMap<Ulid, EntityId>`, and an initialized
  flag, with a private adapter method for focused unit testing and no
  observation handles, World snapshots, trace logs, or printing. It consumes
  T016c's version-1 `{"combat":1,"op":"init"|"attack"}` grammar exactly with
  its eight-step first-failure-wins precedence (non-object; version;
  op; lexically smallest unknown field; required ULIDs in order with
  `must be a ULID string` vs `invalid` split and core alias acceptance;
  duplicate-init/not-initialized; unknown encounter then actor-before-target
  identities with canonical Display; sim `init failed:`/`attack failed:`
  Displays verbatim), binds via the transient borrowed `EncounterSpec` over
  `start_encounter` plus the T016c placement/ability binding assertions
  (publishing only on success, preserving `missing ...` and binding-mismatch
  reasons), and attacks via `perform_action` with `CombatAction::UseAbility`
  discarding the outcome with `.map(|_| ())`. No World mutation occurs beyond
  those two public sim calls; no winner or final-health check lives here.
  Testkit's `tests/support/combat.rs` is a worked example, never an importable
  runtime API (no `include!`, no new export, no fixture-count/package-name/
  attribute/seed/winner checks in CLI production code).

## Combat replay invariants (T016d)

1. **Two modes, one flag.** The mode decision is the explicit presence of
   `--campaign`, never content sniffing or a path special case. Legacy
   semantics and the payload-agnostic testkit boundary (`play_and_verify`
   owns read/validation/interleaving/comparison) remain in force.
2. **Verify-only, no rebless.** No `--write`, recording, resolver, damage,
   turn loop, runtime OS golden selection, or rebless mode. Target-scoped
   goldens are selected explicitly (or as siblings) and compared exactly.
3. **Data owns loading/validation; sim owns combat; the CLI owns process.**
   No kind tables, reference walks, diagnostic sorts, second diagnostic shape,
   combat resolution, dice, damage, turn, or replay semantics live here.
4. **No new dependencies.** Existing edges to core/data/sim/testkit plus
   `serde_json` suffice; the engine version type is inferred and the action
   result is discarded without naming rules-owned types. No manifest,
   lockfile, public library facade, lower-crate, `src/apply.rs`, or
   `tests/replay.rs` change; existing `main.rs` parser tests gain only the new
   field and the extended usage string with assertions retained, never
   weakened.
5. **Tests are black-box first, seams second.** `tests/combat_replay.rs`
   drives `env!("CARGO_BIN_EXE_crpgc")` over private temp copies with std-only
   temp conventions; checked-in inputs stay read-only and no test generates a
   golden. `main.rs`/`combat_apply.rs` unit tests cover the parser matrix,
   injected engine/collector errors, semantic rendering (including a synthetic
   warning), and adapter rollback by comparing full World bytes and binding
   state. Native success/divergence tests use T009c's compile-time cfg guards;
   portable usage/collection/payload/error tests never read a scoped baseline,
   never runtime-select, tolerate, skip, or assert cross-platform equality.

## Second-ruleset CLI proof (T017f, specified before source)

Same thin-adapter shape for `combat_srd`: `src/combat_apply.rs` gains only
the `end`-op decode (`{"combat": 1, "op": "end", "actor": <ULID>}` onto
`CombatAction::EndTurn`) with error-string parity to T017e's worked
example, guarded by one private pure result-shape helper
(`require_end_outcome`: `None` accepted, `Some` reported, never silent).
No parser, collector, loader, flag, exit-code, or stream-convention change.
`tests/srd_replay.rs` (reserved by T017) proves both campaigns verify
through the identical `replay --campaign --golden` command shape outside
the repository working directory, over B4's read-only replay and scoped
goldens; `tests/combat_replay.rs` stays untouched. Tested inputs stay
read-only; no test generates a golden or mutates the source tree.

As-built: 10-test `srd_replay` suite green on both natives (native golden
match, sibling default, both option orders, spaced/renamed campaign plus
unrelated cwd, both-campaigns-same-command, tick-9 divergence, dead/out-of-
turn application failures, end-error matrix, verify-only trees); 5 new
`combat_apply.rs` unit tests (result-shape guard, decode precedence,
guards/rollback, accepted end without RNG draw, srd dispatch); B4
artifacts reused read-only.

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
- **Migrate tests are black-box first, writer seams second.**
  `tests/migrate.rs` drives `env!("CARGO_BIN_EXE_crpgc")` over private temp
  copies of T012a's `migration_v1/campaign` and compares the full resulting
  file-to-byte map with the checked-in `expected.json` golden; the source
  tree is never migrated and expected bytes are never generated from actuals.
  `main.rs` unit tests cover only the private parser matrix, the
  `plan_updates`/key-set seam, the `RewriteFs`-injected rewrite seams
  (ancestor/target symlinks, changed source, missing/nonregular target,
  open/write/sync failures, first-lexical-failure stop, zero-write on empty
  updates and on preflight failure, prefix-retained bounded partial writes),
  and the `emit_code` stream-failure seam. Real-symlink and non-Unicode
  process tests are compile-time cfg-gated; permission shapes use injected
  `ErrorKind`s, never runtime skips. Second-migration zero writes are pinned
  through the writer seam, not timestamps.
- **No `--golden` defaulting into the fixture set.** The default golden is a
  sibling of the replay path, never a search through testkit's goldens. A
  CLI run must verify what it is told to verify.
- **T013 usage is one line per command, never an echo.** New-command usage
  failures print exactly `crpgc: usage: <syntax>\n` with the interface-block
  line; tests assert the bytes, not a substring, so a helpful echo would
  break the contract it claims to document.
- **New-command text is Unicode; paths are `OsString` to the last moment.**
  Slug/id/type/number parsing rejects non-Unicode as usage (exit 2), while
  roots and catalogs parse and fail later as exit-1 `io`. Conflating the
  two moves a domain failure into usage or panics on lossy conversion.
- **Lock never calls `load_campaign`.** Absent or stale `campaign.lock` is
  the operation's reason to exist; a whole-campaign load would turn that
  into a prerequisite failure. Catalog decoding goes straight to
  `Vec<PackageCandidate>` — a `Value` detour would discard the duplicate
  keys strict decoding must reject.
- **A supplied root may not traverse a symlinked ancestor.** The collector
  checks lexical ancestors before touching the terminal root, matching the
  lock path discipline. This applies to validate, migrate, explain and fmt;
  accepting `alias/campaign` where `alias` is a symlink would make later
  read or rewrite guarantees meaningless.
- **Lock input and output paths must stay distinct.** After all lock preflight
  work and before any output write, existing paths are compared by filesystem
  identity through `same-file`;
  `campaign.lock` aliasing `campaign.json`, `assets/assets.lock`, or the
  catalog, including through a hard link, is `source_changed` with no
  mutation. Do not replace this with textual or canonical-path equality.
- **Campaign rewrites must not alias another campaign-tree file.** Collection
  keeps every observed regular path, including ignored files, and migrate/fmt
  save compares each differing target by filesystem identity before the first
  write. A hard link to ignored content or another document is
  `source_changed`; fmt check remains read-only and does not need this
  write-safety preflight.
- **The trial responses are fixtures, not oracles.** Both
  `tests/inputs/llm-trials/` and `tests/inputs/llm-trials-r2/` hold preserved
  accepted LLM bytes. The rerun test first compares each file to an
  independently transcribed byte constant, then installs it in a private
  fixture copy with the locale-key adapter and requires `validate --json` to
  return `[]\n`. Editing a response to "fix" validation is blessing, not
  acceptance.

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
