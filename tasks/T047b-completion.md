# T047b completion record

Branch `task/T047b-s1`, 2026-11-02 (UTC), Windows (`x86_64-pc-windows-msvc`), Sonnet 5.5.

## Engine and ladder

- Engine: `Godot4 --version` = `4.7.2.stable.official.ed1daf0bf` (ADR-0001 pin).
  gdext reported `API v4.7.stable.official, runtime v4.7.2.stable.official, safeguards strict`.
- Build-ladder step used: **step 1** (the ADR manifest as written:
  `godot = { version = "=0.5.5", optional = true, features = ["api-4-7"] }`,
  default features on). Steps 2 and 3 were not needed.
- Cold `cargo build -p crpg-godot --features engine -j 2` (godot tree plus its
  codegen compiled, to the first error): 88 s, peak 3518 MB (sum of working
  sets of `cargo`/`rustc`/`link` sampled every 0.5 s). The build then
  succeeded in 7 s incrementally (peak 1959 MB). Well under 30 min and no OOM.

## Lock (D3)

13 additions, no removals, no existing version changed
(`git diff -U0 mgr/session-32-batch -- Cargo.lock` has no removed line):
`gdextension-api` 0.5.1, `glam` 0.32.1, `godot` 0.5.5, `godot-bindings` 0.5.5,
`godot-cell` 0.5.5, `godot-codegen` 0.5.5, `godot-core` 0.5.5, `godot-ffi` 0.5.5,
`godot-macros` 0.5.5, `heck` 0.5.0, `nanoserde` 0.2.1, `nanoserde-derive` 0.2.1,
`venial` 0.6.1; plus `"godot"` in `crpg-godot`'s own dependency list.

## `.gdextension` keys that worked on 4.7.2

Exactly the contract text: `entry_symbol = "gdext_rust_init"`,
`compatibility_minimum = 4.7`, `reloadable = false`, and
`windows.debug.x86_64 = "res://../../target/debug/crpg_godot.dll"` (the
`res://../..` path loads). `--import` writes `.godot/` (needed before `-s`:
not separately verified without it) and `*.uid` files, which are not committed.
`--import` exited 0 five times; once, in a first fresh run, the shell printed
`Segmentation fault` after "Loading docks..." (editor shutdown, observed once,
not reproduced in four later runs).

## E1

- `-s res://tests/load.gd` exits 0 (`load.gd OK`). The control,
  `CRPG_CAMPAIGN=C:/no/such/dir`, exits 3 (`open = 20 (CAMPAIGN_IO: open root <campaign-root>)`).
- Reading of the contract: E1 says `open(dir,"","",1)` then `option_labels().size() > 0`
  after one `poll()`. On the landed core the default side is Hero while the goblin
  opens the fixture, so the first poll has no options (`core/tests.rs:160`). `load.gd`
  keeps `open(dir,"","",1)`, asserts revision 1 and the two names on the first
  poll, then polls (50 ms `OS.delay_msec`, up to 5 s, each poll must be 0) until the
  NPC's 600 ms delay passes and the Hero has options.

## B17 observation (thread)

From a `Thread` in GDScript, `session.poll()` on the dev profile (safeguards
strict) printed `ERROR: [panic godot-ffi-0.5.5/src/binding/single_threaded.rs:140]
attempted to access binding from different thread than main thread ... in
CrpgSession::poll()`. gdext's guard fires at the `#[func]` call boundary
before the body: the call is aborted (`Bug: Invalid call error code 1337`),
and the main-thread session was unaffected (`poll` afterwards returned 0).
Release (balanced) was not measured.

## Clippy allows

None. `cargo clippy ... --features engine --all-targets -- -D warnings` is clean.
The only fix needed was a redundant closure.

## `cargo tree -p crpg-godot -i godot` (default features)

```
error: package ID specification `godot` did not match any packages
```

## `cargo deny check` (preflight, workspace graph)

`advisories ok, bans ok, licenses ok, sources ok`. Without `[graph]` (T047c's
change) `deny` does not resolve the optional `godot` family, as the contract
expected, so no MPL-2.0 failure appears yet.

## K31 and K32

K32 added (`the_crate_has_exactly_one_unsafe_on_the_extension_impl`). K31's
clock/float/hash ban was restricted from all of `src/` to `src/core/**`, as
T047 K31 and D5 specify; the shell reads `Instant`. No assertion was removed.

## Gates run

`cargo fmt --all`; `cargo clippy -p crpg-godot --all-targets --locked -j 2 -- -D warnings`;
`cargo test -p crpg-godot --locked -j 2` (44 unit, 2 bridge, 2 source_rules);
engine build, `cargo clippy ... --features engine --all-targets --locked`,
`--locked` engine build, `--import`, E1 and its control;
`deps.py`, `determinism.py`, lint unittests (81); `tools/preflight.ps1 -Crate crpg-godot`.

Agent log:

- 2026-11-02 (UTC) · claude-code/sonnet-5.5 + T047b · Recorded the build ladder, lock, `.gdextension` keys, E1 result and its first-poll reading, the B17 thread observation and the gate results.
