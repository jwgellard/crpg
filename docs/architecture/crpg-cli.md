# crpg-cli — architecture

The `crpgc` toolchain command-line: validate, migrate, pack, run, replay and
diff — headless, no Godot, no rendering.

**State:** the replay subcommand is live (T009b, 2026-09-07): `crpgc replay
<path> [--golden <path>]` reads a versioned replay and its target-scoped
golden, plays through `crpg-testkit::play_and_verify`, and exits `0` on
equality, `1` on any typed replay failure with the single-pathed diagnostic on
stderr, `2` on a usage error. It is a thin consumer: no replay semantics live
in this crate. The combat replay mode is live (T016d): `crpgc replay
<replay-path> [--golden <golden-path>] [--campaign <campaign-root>]` keeps the
legacy reference adapter when `--campaign` is absent and selects the private
combat adapter — production campaign collection, `load_campaign`, empty
`validate`, then `play_and_verify` — when it is present, with the same
`0/1/2` buckets. The validate subcommand is live (T011b, 2026-09-13):
`crpgc validate <campaign-root> [--json]` deterministically collects campaign
document bytes, calls T011a's data-owned `validate_files`, and renders
human or canonical machine diagnostics with the same `0/1/2` exit buckets.
The migrate subcommand is live (T012b, 2026-09-18): `crpgc migrate
<campaign-root>` is the explicit save action for S §4.5 over T012a's
migration-aware loader and canonical writer, silent on success with the same
`0/1/2` buckets. The six T013 commands are live (2026-09-19): `crpgc new`,
`crpgc schema`, `crpgc explain`, `crpgc fmt`, `crpgc lock`, and `crpgc run`
cover scaffolding, introspection, canonical formatting, package locking, and
the bounded hash harness, each with the same `0/1/2` buckets. Every other
subcommand is planned (the T013 parser decision — hand-rolled `args_os` with
private per-command parsers, no parser library — now applies crate-wide).

Scope note (moved from `crates/crpg-cli/AGENTS.md`): this crate describes the
T009b opening (`replay` subcommand, provisional reference-intents apply, exit-code
contract) plus the T011b `validate` thin wrapper (read-only traversal ownership,
gate-8 fixture gate), the T012b `migrate` explicit-save wrapper (preflight/rewrite
boundary, data-golden gate), the six T013 thin wrappers (`new`, `schema`,
`explain`, `fmt`, `lock`, `run`) with their retained hand-rolled per-command
parser decision and black-box plus literal-LLM-trial acceptance, and the T016d
combat replay mode (`--campaign`, private combat adapter, black-box acceptance).
Later subcommands (`pack`, `diff`) extend the crate with their tasks.
Decisions: the platform and determinism policy it inherits is
[ADR-0012](../adr/0012-windows-primary-platform.md) (exact-build target
scopes) over [ADR-0009](../adr/0009-determinism-scope.md); the divergence
shape it prints is [ADR-0010](../adr/0010-testkit-mismatch-shape.md). Working
contract: [`crates/crpg-cli/AGENTS.md`](../../crates/crpg-cli/AGENTS.md).

---

## Position

`crpg-cli` is the human-and-CI face of the simulation core. It sits above
sim, data, rules and testkit and is the one crate the dependency lint grants
an open allow-list (everything except the Godot bridge) — tooling legitimately
touches every layer, but the bridge exclusion stands so a CLI build never
drags Godot/windowing/audio into CI. Today it uses `crpg-testkit`
(`play_and_verify`), `crpg-sim` (`World`) and `crpg-core` (`EntityId`)
through the apply, plus workspace `serde_json` for payload access.

Its job is narrow: map command lines to crate APIs and map results to exit
codes. Behavioural authority stays in the lower crates; the CLI may call
them in new combinations, never reimplement their semantics. Any replay
logic that would have to live here proves testkit's API is incomplete —
file it there, do not reach around.

## Modules

- **`main` — dispatch, arguments, exit codes.** One variant per subcommand.
  Argument parsing: hand-rolled `std::env::args_os` organized into private
  per-command parsers crate-wide (T013 retained this decision for all six new
  commands; no parser library is authorized). Exit codes follow the
  established `0` success, `1` domain failure, `2` usage convention:
  `0` success, `1` replay-domain failure, `2` usage error.
- **`apply` — provisional reference intents.** The caller-owns-payload rule
  (testkit invariant 8) means a replay's `serde_json::Value` payloads mean
  whatever the *caller* says. Until a real game-intent task (T014/T016)
  defines them, this module interprets the same miniature reference
  vocabulary the checked-in T009a fixture was generated against —
  `spawn_at`, `timeline`, `draw`, `despawn` — over public `World` APIs only.
  It exists so `crpgc replay` can verify that fixture against its native
  golden; it is superseded, not blessed, when real intents land, and nothing
  outside this crate may import it.
- **`combat_apply` — private combat intents (T016d).** The caller-owned
  counterpart for the versioned `{"combat":1,"op":"init"|"attack"}` grammar:
  `pub(crate) fn combat_intents(LoadedCampaign) -> ApplyInput` returns an
  owned `move` closure over the loaded documents, a placement-to-entity map,
  and an initialized flag. It decodes/binds through a private adapter method
  onto exactly `start_encounter`/`perform_action` with T016c's error
  precedence and binding assertions, discarding the action outcome. No
  observation handles, snapshots, traces, or printing; no combat, replay, or
  validation semantics are duplicated here.

## CLI contract (T009b)

```
crpgc replay <replay-path> [--golden <golden-path>]

  <replay-path>   .replay file to read and validate.
  --golden <path> golden to compare against. Default: replay-path with its
                  extension replaced by `.golden`.

exit 0  sequence matches the golden. Nothing is printed; this is a
        verification tool, not a recorder.
exit 1  any crpg-testkit::ReplayError, printed by its single-pathed Display
        to stderr: Io (a missing replay or golden is distinguishable from a
        divergence), Malformed, UnsupportedVersion, UnorderedSchedule,
        OutOfRange, TooLarge, ApplyFailed, Divergence (exact-tick report).
exit 2  usage error: missing/unknown subcommand, missing replay path,
        unknown flag, missing --golden value.
```

`--golden` is default-through-extension, not default-through-`play`-and-`read`:
omitting it is a path decision, never a "use any golden" trap.

These exit codes are a public CI-facing contract, stable from the first
version.

## CLI contract (T016d combat `replay --campaign`)

```
crpgc replay <replay-path> [--golden <golden-path>] [--campaign <campaign-root>]

  <replay-path>    .replay file to read and validate.
  --golden <path>  golden to compare against. Default: replay-path with its
                   extension replaced by `.golden`.
  --campaign <dir> campaign tree supplying combat content; selects combat
                   payload semantics. Absent: the legacy reference adapter.

exit 0  sequence matches the golden. Nothing is printed; this is a
        verification tool, not a recorder.
exit 1  any campaign collection/load/semantic failure or any
        crpg-testkit::ReplayError, printed by its single-pathed Display
        to stderr with the `crpgc replay: ` prefix for replay errors.
exit 2  usage error: missing replay path, flag before the replay path,
        unknown flag, missing or duplicate `--golden`/`--campaign` value,
        extra positional, `--campaign=...`, `--`, or recording syntax.
```

The replay path stays first; `--golden` and `--campaign` may follow in either
order, at most once each, with separate-token values. `--campaign` is the
only mode selector: no sniffing, name special case, search, fallback, or mixed
vocabulary. Combat without the flag reaches the reference unknown-intent
error; reference payloads with the flag fail combat version decoding.

## Combat replay flow and boundaries (T016d)

Combat mode runs parse (whole command before any I/O; non-Unicode campaign
roots parse for later exit-1 `io`) → collect the supplied tree through the
existing `collect_campaign_files_with`/`RealFs` discipline → `load_for_command`
with the package engine version calling `load_campaign` once (defensive
version failure `crpgc replay: internal engine version failure`, exit 1) →
`crpg_data::validate(&loaded)` once requiring the empty list (strict: warnings
also fail here; findings print in returned order via the existing
diagnostic-line formatter) → move the campaign into `combat_apply::
combat_intents` and call `play_and_verify` once. Precedence is usage →
collection → engine/load → semantic findings → replay read/validation →
application → golden comparison, so a bad campaign beats a missing replay and
an invalid action beats a missing golden. Only `start_encounter` and
`perform_action` mutate `World`; metadata stays descriptive and the golden
covers the resolved content. No resolver, damage, turn loop, runtime OS
selection, rebless, fixture-count/package-name checks, or second playback
loop lives here.

## Combat replay detailed contract (T016d, moved from AGENTS.md)

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

## Combat replay second ruleset (T017f, specified before source)

The combat adapter gains only the `end`-op decode onto
`CombatAction::EndTurn` with error-string parity to the testkit worked
example, guarded by one private pure result-shape helper; the command,
flags, parser, and golden handling stay unchanged. `tests/srd_replay.rs`
proves both campaigns verify through the identical command shape outside
the repository working directory, over read-only B4 replay and goldens.
No winner, health, pool, attachment, or turn observation lives in
production adapter code.

As-built: the `end` decode with `require_end_outcome`, the 10-test
black-box suite pinning golden matches, divergence, application failures,
and verify-only trees, and unchanged `combat_basic` coverage.

Second-ruleset CLI proof (moved from AGENTS.md): same thin-adapter shape for
`combat_srd`: `src/combat_apply.rs` gains only the `end`-op decode
(`{"combat": 1, "op": "end", "actor": <ULID>}` onto
`CombatAction::EndTurn`) with error-string parity to T017e's worked
example, guarded by one private pure result-shape helper
(`require_end_outcome`: `None` accepted, `Some` reported, never silent).
No parser, collector, loader, flag, exit-code, or stream-convention change.
`tests/srd_replay.rs` (reserved by T017) proves both campaigns verify
through the identical `replay --campaign --golden` command shape outside
the repository working directory, over B4's read-only replay and scoped
goldens; `tests/combat_replay.rs` stays untouched. Tested inputs stay
read-only; no test generates a golden or mutates the source tree.

As-built (moved): 10-test `srd_replay` suite green on both natives (native golden
match, sibling default, both option orders, spaced/renamed campaign plus
unrelated cwd, both-campaigns-same-command, tick-9 divergence, dead/out-of-
turn application failures, end-error matrix, verify-only trees); 5 new
`combat_apply.rs` unit tests (result-shape guard, decode precedence,
guards/rollback, accepted end without RNG draw, srd dispatch); B4
artifacts reused read-only.

## Today versus planned

| Exists (T009b/T011b/T012b/T013) | Planned (owner) |
|---|---|
| `crpgc replay` thin wrapper: args, exit codes, provisional intents | Spec §24 remaining subcommands (`pack`, `diff`) as their specs land |
| `crpgc replay --campaign` combat mode (T016d): explicit mode flag, production campaign load plus empty validate, private combat adapter over `start_encounter`/`perform_action`, black-box acceptance | Spec §24 remaining subcommands (`pack`, `diff`) as their specs land |
| `crpgc validate` thin wrapper (T011b): read-only traversal, data-owned validation, plain/canonical diagnostics, gate-8 fixture enumeration | Spec §24 remaining subcommands (`pack`, `diff`) as their specs land |
| `crpgc migrate` thin wrapper (T012b): explicit save, preflight plus bounded rewrite, data-golden tests | Spec §24 subcommands (`pack`, `diff`) as their specs land |
| `crpgc new`/`schema`/`explain`/`fmt`/`lock`/`run` thin wrappers (T013): explicit-identity scaffolds, data-generated schemas, data-owned introspection reports, canonical check/save, flat-catalog lock adapter, bounded no-op harness sampling | Spec §24 subcommands (`pack`, `diff`) as their specs land |
| Crate-opening docs (arch doc + AGENTS.md) | Spec §24 subcommands (`pack`, `diff`) as their specs land |
| Verify-only replay gate on ADR-0012 targets | `crpgc` in product/driver roles once server capabilities exist (E020) |

## CLI contract (T011b `validate`)

```
crpgc validate <campaign-root> [--json]

  <campaign-root>  directory containing campaign.json
  --json           emit one canonical JSON diagnostic array to stdout

exit 0  campaign loads and has no Severity::Error diagnostic
exit 1  I/O, structural load, or semantic Error failure
exit 2  usage error
```

`--json` may appear once either before or after the root. Duplicate flags,
extra positionals, unknown flags, and a missing root are usage errors
(exit 2). Warnings print
(plain) or are included (JSON) but never fail: exit stays `0` when no `Error`
is present. The engine version is the compile-time Cargo package version
parsed as semver; an unparsable version is exit 1 with a fixed internal
line, never a panic. That is one of two fixed-text internal failures (exit
1); the other is a canonical serialization failure. These are the same three
exit buckets as `replay`, extended, not replaced.

## Validate flow and boundaries (T011b)

The implementation flow is exactly parse (`args_os`, so a non-Unicode root
never panics) → collect classified files in sorted depth-first pre-order →
`crpg_data::validate_files(files, package engine version)` → render the
returned diagnostics → map no-Error/any-Error to `0/1`. Filesystem traversal
lives in `main` behind a `WalkFs` seam (`std` only, read-only, never follows
symlinks, `/`-joined logical paths, `campaign_document_path` as the only
classifier); every validation semantic lives in `crpg-data`, unchanged and
un-duplicated here — no kind tables, graph walks, diagnostic sorts, or second
diagnostic shape. The `WalkFs` trait exists so tests can inject unreadable
files, symlinks, non-Unicode names, and walk-order oracles without platform
privileges; production implements it with `symlink_metadata`, unordered
`read_dir` (the walker sorts by file-name bytes), and plain reads.

Plain mode prints one data-owned `Display` line per diagnostic to stderr in
returned order (stdout empty; a clean run is silent). JSON mode writes the
complete vector through `crpg_data::canonical_json` to stdout (`[]\n` when
clean; stderr empty). Collection `io` failures are the single CLI-owned
diagnostic shape — always `Error`, empty pointer, no fix, portable
`cannot <op> <logical>: <kind>` text with no raw OS detail — while
classifier rejections pass through `diagnostic_for_data_error` unchanged as
`invalid_path`. Gate 8 is the ordinary black-box test over T011a's
data-owned `expected.json` manifest: manifest roots must equal the
discovered `campaign.json` roots exactly, and each root is invoked through
the shipped binary (`clean` → exit `0` with exact `[]\n`; `diagnostics` →
exit `1` with stdout byte-equal to the checked-in snapshot). A future
fixture-owning task extends the manifest in `crpg-data`; this test picks it
up generically with no CLI edit.

## Validate invariants (T011b, moved from AGENTS.md)

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

## CLI contract (T012b `migrate`)

```
crpgc migrate <campaign-root>

  <campaign-root>  directory containing campaign.json

exit 0  all required canonical source replacements completed, or no change needed
exit 1  collection, structural/migration, serialization or write failure
exit 2  usage error
```

Exactly one root; a missing argument, an extra positional, or any flag
(including `--json`, `--check`, `--dry-run`, `--to` and `--golden`) is usage
exit 2 with one `crpgc: ...` line on stderr. A supplied-but-missing,
unreadable, or non-Unicode root is a domain exit-1 `io` diagnostic, not usage.
Success is silent with both streams empty, including a second no-op
invocation; stdout is always empty. Domain failures print one
data-owned `Diagnostic::Display` line or one CLI-owned `io` line to stderr.
The compile-time engine version parses exactly as T011b does with no semver
edge; an unparsable version prints `crpgc migrate: internal engine version
failure` and a writer key-set mismatch prints `crpgc migrate: internal
document set failure`, both exit 1 before any write.

## Migrate flow and boundaries (T012b)

The explicit save flow is parse (`args_os`) → the existing T011b read-only
collector (sorted depth-first pre-order, root-ancestor/root/entry symlink rejection, `/`
logical paths, `campaign_document_path` classification, ignored-file policy)
→ `load_campaign(files, package engine version)` once → `serialize_campaign`
once → compare returned bytes with collected originals → replace differing
recognized files in lexical `SourcePath` order. All collection, load, and
serialization work succeeds for the entire map before any write starts, so
preflight failures (incompatible engine, unknown future schema, broken chain,
malformed payload, invalid layout, duplicate id, invalid lock, digest
mismatch) perform zero writes. The command never calls `validate_files`,
individual migration steps, or any semantic check; a structurally valid
campaign with T011a findings migrates successfully and keeps its exact
subsequent validate snapshot. Version chains stay in data; the CLI never
parses a version, changes a schema tag, decodes campaign JSON, recomputes a
digest, or implements a semantic check.

The save serializes all collected documents through the data writer,
including locks, and writes only byte-different files at their existing
logical paths, so noncanonical whitespace/ULID spelling may normalize on
explicit save even when its schema was already current. Canonical-current
files are never opened. No renames, deletions, new documents, source assets,
copied schemas, Lua, build output, or replay changes; ignored content stays
byte-identical. For each differing file the CLI rechecks root/ancestor/target
metadata without intentionally following symlinks, re-reads the target and
requires equality with the collected original (`source_changed` on drift,
`not_a_file` for a directory/other target), then opens the existing file
with truncation, `write_all`, and `sync_all` without create-on-missing
semantics, stopping at the first failure. Rewrite diagnostics reuse the
existing six-field `Error`/`Io` shape with `cannot <op> <logical>: <kind>`
text (`check`, `open`, `write`, `sync`; T011b kinds plus `source_changed` and
`not_a_file`); ancestor failures name the target logical path.

Bounded partial-write and live-tree limitations: this deliberately does not
promise atomic replacement, rollback, crash recovery, or a whole-directory
transaction. An I/O failure can leave earlier files replaced and the failing
file partially written. As with collection, std metadata/recheck is not
handle-based no-follow protection; concurrent changes after a check remain
unspecified. No locking or secure live-tree snapshot is introduced. A future
atomic save design is separate scope. Stream-write failures exit 1 without
panic and without recursively reporting the broken stream.

Golden tests copy T012a's `migration_v1/campaign` to private temp dirs,
invoke the shipped binary, and compare the full resulting file-to-byte map
with the sibling `expected.json` path-to-text golden; the checked-in source
tree is never migrated and expected bytes are never generated from actual
output in tests.

## Migrate invariants (T012b, moved from AGENTS.md)

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

## CLI contracts (T013 `new` / `schema` / `explain` / `fmt` / `lock` / `run`)

```
crpgc new <type> --slug <s> --id <id> [--entry-id <id>]
crpgc schema <type>
crpgc explain <id> [--root <campaign-root>]
crpgc fmt [<campaign-root>] [--check]
crpgc lock [<campaign-root>] --catalog <catalog-path>
crpgc run --ticks N --hash-every M [--seed S]
```

`explain`, `fmt` and `lock` default their root to `.` with no ancestor
search, environment override, implicit stdin, or repository-root lookup.
Relative paths, including the catalog, are relative to the process working
directory. Parsing completes before any filesystem access or output: each
named option occurs at most once before or after positionals, values are
separate tokens (never `--key=value`), and unknown flags, duplicates,
missing values, extra positionals, `--`, short flags, and unlisted
`--help`/`--version` forms are usage exit 2 with exactly
`crpgc: usage: <syntax>\n` for the recognized command — one message for all
its usage errors, never echoing input. Text arguments must be Unicode; path
arguments stay `OsString` until I/O validation, so a non-Unicode path is
exit 1, never exit 2. Success output is UTF-8 with LF. All success output is
silent except `new` (one canonical document on stdout), `schema` (exact
data-generated schema bytes on stdout), `explain` (`Some(bytes)` on stdout),
and `run` (sampled hash lines on stdout).

## T013 detailed command contracts (moved from AGENTS.md)

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

## Parser decision (T013)

The hand-rolled `std::env::args_os` parser is retained and organized into
private per-command parsers. A parser library would be useful at this
command count, but no clap or other new dependency is authorized, and
adopting one would risk changing established error bytes and non-Unicode
handling. The bounded grammar plus exhaustive parser/process tests justify
the decision; the task file records that the draft is not dependency
approval.

## Scaffolding and introspection boundaries (T013)

`new` takes explicit caller-supplied identities (`--id` always, plus a
distinct `--entry-id` for dialogue/quest; forbidden otherwise) with the
slug grammar `[a-z0-9]+(?:-[a-z0-9]+)*` checked without a regex crate, then
builds the existing public typed `Document` variants and calls
`write_document` — never handwriting envelopes, schema tags, or canonical
JSON. No clock, RNG, registry, or filesystem lookup; the author supplies
locale entries and file placement, and repeated calls with identical
operands produce identical bytes.

`schema` writes `generated_schemas()` bytes for the requested stem
unchanged; `explain` parses the id with `Ulid::from_str`, collects, calls
`load_campaign` once and the landed `explain_object` once, and emits
`Some(bytes)` untouched — with `None` as exit 1
`crpgc explain: object not found: <canonical-uppercase-id>\n` and no CLI
reference walker, kind table, or semantic-validation prerequisite. `fmt`
reuses migrate's collect/load/serialize/key-set/diff plan with the T012b
writer discipline (`--check` is the same preflight with
`crpgc fmt: noncanonical: <logical>\n` per differing file and no writes).
`lock` reads `campaign.json`, `assets/assets.lock`, and the flat JSON-array
catalog in that order, resolves with `make_campaign_lock`, serializes with
`write_campaign_lock`, and creates (via `create_new` with `source_changed`
on races), replaces, or no-ops `campaign.lock` — never reading source
assets, rewriting `assets.lock`, or calling `load_campaign`. Before output,
existing paths are compared by filesystem identity through the
maintainer-authorized `same-file` dependency so `campaign.lock` cannot also be
the catalog, campaign manifest, or assets lock, including through a hard
link; such an alias is `source_changed` with no mutation. `run` calls
`run_hash_sequence(seed, ticks, Box::new(|_| {}))` once and prints only the
`M, 2M, ... <= N` samples as `<k> <64-lowercase-hex>\n`; it is a bounded
harness, not campaign execution.

Migrate and fmt save retain every regular path observed during collection,
including ignored files, and compare each differing target against that set
through `same-file` before the first write. This rejects a document hard-linked
to ignored content or another document as `source_changed`; fmt check remains
a strictly read-only preflight and skips the write-only identity check.

Bounded writes everywhere: saves stop at the first failure with no atomic
replacement, rollback, crash recovery, or directory transaction, and may
leave a partial prefix. Stream-write failures exit 1 without panic or
recursion.

## Test and acceptance arrangement (T013)

Binary integration suites `tests/scaffolding.rs`, `tests/explain.rs`,
`tests/fmt.rs`, `tests/lock.rs`, and `tests/run.rs` drive
`env!("CARGO_BIN_EXE_crpgc")` with std-only temp conventions; `main.rs`
unit seams pin every parser matrix plus injected I/O/stream/internal
failures. Scaffolds carry independently specified expected bytes and must
validate clean inside private fixture copies; all seventeen stems compare
against the read-only checked-in `schemas/`; `fmt` save must equal the
migration golden; lock creation/replacement/no-op and sampled harness
output are pinned through real data/harness calls, never a CLI oracle. Four
literal schema-only accepted LLM trials (creature/item/dialogue/quest,
recorded in `tasks/T013.md`) are preserved in `tests/inputs/llm-trials/` and
`tests/inputs/llm-trials-r2/`. The native rerun test pins every file against
an independently transcribed byte constant before installing it into a
private campaign and requiring `validate --json` success; it is not a
network-dependent CI step.

## Known traps (moved from AGENTS.md)

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

## T013 invariants (moved from AGENTS.md)

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

## Combat replay invariants (T016d, moved from AGENTS.md)

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

## What consumers inherit

- **A scriptable replay gate.** `crpgc replay` in a CI step is the public
  face of the testkit replay gate: exit status + stderr diagnostic carry the
  whole result, so shell scripts compare status rather than re-implementing
  replay comparison.
- **One apply, one fixture, byte-identical.** The CLI's provisional reference
  apply mirrors testkit's `tests/support` example exactly, so the fixture
  verifies identically through the binary and through testkit's own tests.
  When real intents land, this apply is replaced by the owning task's, and
  the fixture it verified against is superseded by the new intent set.
- **No cross-target promises.** The CLI verifies against a golden named for
  its native target (ADR-0012); it never compares Windows hashes to Linux
  hashes, and it owns no re-baseline command — golden changes stay reviewed
  events in testkit.

## Agent log

- 2026-09-26 (UTC) · opencode/muse-spark + T016d documentation · Documented the explicit two-mode combat replay contract (parser, production loading plus empty-validate preflight, private combat adapter, precedence, and verify-only boundaries) before source changes, superseding the reference-only statements while retaining legacy semantics.
- 2026-09-27 (UTC) · opencode/muse-spark + T017f crate opening · Recorded the second-ruleset CLI proof shape (single end-op decode with error-string parity, unchanged command, black-box suite over both campaigns) before source implementation.
- 2026-09-27 (UTC) · opencode/muse-spark + T017f implementation · Aligned the module with the as-built proof (end decode with result-shape guard, 10-test suite, B4 artifacts read-only).

- 2026-09-07 (UTC) · opencode/big-pickle + T009b · Opened the crate with its
  first real code: the thin `crpgc replay` wrapper, the provisional
  reference-intents apply (caller-owns-payload), hand-rolled args with clap
  revisited at T013, and the exit-code contract that makes the CLI a
  scriptable gate.
- 2026-09-13 (UTC) · opencode/muse-spark + T011b implementation · Documented
  the thin `crpgc validate` wrapper: data/OS boundary with traversal
  ownership, plain/canonical output with the 0/1/2 exit contract, gate-8
  manifest enumeration, and the unchanged T013 parser decision.
- 2026-09-18 (UTC) · opencode/muse-spark + T012b implementation · Documented
  the thin `crpgc migrate` explicit save: data-owned version boundary,
  preflight/no-op rules, bounded partial-write and live-tree limitations,
  errors/exit codes, and data-golden tests; moved migrate from planned to
  implemented while preserving T013 ownership of the remaining commands.
- 2026-09-19 (UTC) · opencode/muse-spark + T013 implementation · Documented
  the six live T013 commands: the retained hand-rolled per-command parser
  decision, explicit-identity scaffold and data-owned byte/report/lock
  boundaries, the fmt check/save flow with bounded writes, the harness-only
  run, and the binary-test plus literal-LLM-trial acceptance arrangement.
- 2026-09-19 (UTC) · opencode/muse-spark + T013 review remediation (R7) · Aligned the stale Modules parser paragraph with the landed T013 decision: hand-rolled `args_os` per-command parsers crate-wide with no parser library and no future swap, keeping the established 0/1/2 exit convention.
- 2026-09-22 (UTC) · opencode/gpt-5.6-sol + T013 independent review hardening · Documented lexical root-ancestor checks, normalized lock input/output alias rejection, the stable-std hard-link boundary, and byte-pinned validation of both preserved LLM trial sets after adversarial contract review.
- 2026-09-22 (UTC) · opencode/gpt-5.6-sol + T013 hard-link authorization · Replaced the documented stable-std limitation with the maintainer-authorized `same-file` identity check, covering direct, normalized, case, and hard-link aliases between lock output and all inputs.
- 2026-09-22 (UTC) · opencode/gpt-5.6-sol + T013 fmt alias hardening · Documented fmt's complete regular-file identity preflight, which rejects rewrite targets hard-linked to ignored content or another document before mutation.
- 2026-09-22 (UTC) · opencode/gpt-5.6-sol + T013 shared-writer alias hardening · Extended the documented identity preflight to migrate so the shared writer preserves ignored hard-linked campaign content for both save commands.
- 2026-10-16 (UTC) · opencode-go/beam-free + A0-8 · Absorbed the detailed contracts, per-subcommand invariants, second-ruleset proof, and command-specific traps moved out of the trimmed AGENTS.md; no information removed.
- 2026-10-16 (UTC) · claude-code/opus-5.5 + A0-8 review fix · Restored four facts the trim dropped (replay exit codes as a stable CI-facing contract; validate usage errors, its second fixed-text internal failure, and its three exit buckets), found by a Beam review and a keyword check.
