# crpg-cli — architecture

The `crpgc` toolchain command-line: validate, migrate, pack, run, replay and
diff — headless, no Godot, no rendering.

**State:** the replay subcommand is live (T009b, 2026-09-07): `crpgc replay
<path> [--golden <path>]` reads a versioned replay and its target-scoped
golden, plays through `crpg-testkit::play_and_verify`, and exits `0` on
equality, `1` on any typed replay failure with the single-pathed diagnostic on
stderr, `2` on a usage error. It is a thin consumer: no replay semantics live
in this crate. Every other subcommand is planned (T013 owns the first real
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

| Exists (T009b) | Planned (owner) |
|---|---|
| `crpgc replay` thin wrapper: args, exit codes, provisional intents | T013: first real argument-parsing decision (clap vs hand-rolled again) for the five-plus subcommand surface: `new`, `schema`, `explain`, `fmt`, `lock`, `run` |
| Crate-opening docs (arch doc + AGENTS.md) | Spec §24 subcommands (`validate`, `migrate`, `pack`, `diff`) as their specs land |
| Verify-only replay gate on ADR-0012 targets | `crpgc` in product/driver roles once server capabilities exist (E020) |

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