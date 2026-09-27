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

`--json` may appear once either before or after the root. Warnings print
(plain) or are included (JSON) but never fail: exit stays `0` when no `Error`
is present. The engine version is the compile-time Cargo package version
parsed as semver; an unparsable version is exit 1 with a fixed internal
line, never a panic.

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
