# crpg-cli — architecture

The `crpgc` toolchain command-line: validate, migrate, pack, run, replay and
diff — headless, no Godot, no rendering.

**State:** the replay subcommand is live (T009b, 2026-09-07): `crpgc replay
<path> [--golden <path>]` reads a versioned replay and its target-scoped
golden, plays through `crpg-testkit::play_and_verify`, and exits `0` on
equality, `1` on any typed replay failure with the single-pathed diagnostic on
stderr, `2` on a usage error. It is a thin consumer: no replay semantics live
in this crate. The validate subcommand is live (T011b, 2026-09-13):
`crpgc validate <campaign-root> [--json]` deterministically collects campaign
document bytes, calls T011a's data-owned `validate_files`, and renders
human or canonical machine diagnostics with the same `0/1/2` exit buckets.
The migrate subcommand is live (T012b, 2026-09-18): `crpgc migrate
<campaign-root>` is the explicit save action for S §4.5 over T012a's
migration-aware loader and canonical writer, silent on success with the same
`0/1/2` buckets. Every other subcommand is planned (T013 owns the first real
argument parsing decision).

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
  Argument parsing: hand-rolled `std::env::args` for T009b (one subcommand, one
  optional flag); the clap question is reopened at T013 when the five-plus
  subcommand surface arrives. Exit codes follow clap's convention so the
  eventual swap stays script-compatible: `0` success, `1` replay-domain
  failure, `2` usage error.
- **`apply` — provisional reference intents.** The caller-owns-payload rule
  (testkit invariant 8) means a replay's `serde_json::Value` payloads mean
  whatever the *caller* says. Until a real game-intent task (T014/T016)
  defines them, this module interprets the same miniature reference
  vocabulary the checked-in T009a fixture was generated against —
  `spawn_at`, `timeline`, `draw`, `despawn` — over public `World` APIs only.
  It exists so `crpgc replay` can verify that fixture against its native
  golden; it is superseded, not blessed, when real intents land, and nothing
  outside this crate may import it.

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

## Today versus planned

| Exists (T009b/T011b/T012b) | Planned (owner) |
|---|---|
| `crpgc replay` thin wrapper: args, exit codes, provisional intents | T013: first real argument-parsing decision (clap vs hand-rolled again) for the five-plus subcommand surface: `new`, `schema`, `explain`, `fmt`, `lock`, `run` |
| `crpgc validate` thin wrapper (T011b): read-only traversal, data-owned validation, plain/canonical diagnostics, gate-8 fixture enumeration | Spec §24 remaining subcommands (`pack`, `diff`) as their specs land |
| `crpgc migrate` thin wrapper (T012b): explicit save, preflight plus bounded rewrite, data-golden tests | Spec §24 subcommands (`pack`, `diff`) as their specs land |
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
collector (sorted depth-first pre-order, root/entry symlink rejection, `/`
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