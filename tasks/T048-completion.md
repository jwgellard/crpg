# T048 completion record

Branch `task/T048-i1`, 2026-11-03 (UTC), Windows (`x86_64-pc-windows-msvc`),
DeepSeek V4.1 Flash (opencode-go), attempt 1.

## Red check (harness control)

Run before replacing `SceneSync.gd`:

```
Godot4 --headless --path apps/client -s res://tests/scene_sync.gd
# scene_sync.gd FAIL: S1: plan() is missing
# exit 6
```

The first case checks `has_method("plan")`, so T047d's missing method is a case
failure (exit 6), not a script-load failure (exit 3).

## Definition of done

Run from Git Bash at the worktree root, with `CARGO_TARGET_DIR` and
`GDRUST_MAIN_EXTENSION` unset. Every command and its exit code:

| Command | Exit |
|---|---|
| `cargo fmt --all` | 0 |
| `cargo clippy -p crpg-godot --all-targets --locked -j 2 -- -D warnings` | 0 |
| `cargo test -p crpg-godot --locked -j 2` | 0 (45 unit, 2 bridge, 2 source-rules) |
| `python tools/lint/deps.py` | 0 |
| `python tools/lint/determinism.py` | 0 |
| `python -m unittest discover -s tools/lint -p "test_*.py"` | 0 (81 tests) |
| `cargo clippy -p crpg-godot --features engine --all-targets --locked -j 2 -- -D warnings` | 0 |
| `cargo build -p crpg-godot --features engine --locked -j 2` | 0 |
| `pwsh -NoProfile -File tools/preflight.ps1 -Crate crpg-godot` | 0 |
| `Godot4 --headless --path apps/client --import` | 0 |
| `Godot4 --headless --path apps/client -s res://tests/scene_sync.gd` | 0 (`scene_sync.gd OK`) |
| `Godot4 --headless --path apps/client -s res://tests/load.gd` | 0 (`load.gd OK`) |
| `CRPG_CAMPAIGN=/no/such/dir ... -s res://tests/load.gd` | 3 (`open = 20`) |
| `Godot4 --headless --path apps/client -s res://tests/smoke.gd` | 0 (`smoke.gd OK`) |
| `CRPG_CAMPAIGN=/no/such/dir ... -s res://tests/smoke.gd` | 3 (`open = 20`) |
| `git diff --check` (after removing the 8 `*.uid` files) | 0 |

`cargo test` ran R1: `core::client::tests::a_leave_shrinks_every_column_and_a_retired_id_cannot_return ... ok`.

## Readings

1. **Mesh dimensions are 32-bit.** `CapsuleMesh.radius` and `.height` read
   back as `0.40000000596046` / `1.20000004768372`, so the case compares them
   with `is_equal_approx`; every `Vector3` field still compares exactly.
2. **S13's Main.gd control.** `Main.gd` calls `next.revision()`, not
   `view.revision()`, so the literal `view\.` regex finds no `revision` there.
   I read "the view call `revision`" as the `revision` method called on a
   view-typed value and assert the banned token `session` plus `.revision(` on
   `Main.gd`, which keeps the control non-vacuous.
3. **Runner load failure.** `load()` returns a non-null `GDScript` for a file
   with parse errors, and `new()` on it aborts `_initialize`; without a guard
   the runner would hang. Both the runner and the smoke hook check
   `can_instantiate()` (and `has_method("run")`) first. A deliberately broken
   case file exits 3, not a timeout.
4. **Dead material identity.** A dead row reuses its `StandardMaterial3D`, so
   B8's override comparison sees the same object on a repeated sync; the tint
   is still written unconditionally.

## Size

`SceneSync.gd` 120 lines (budget 90–120), `smoke.gd` +8 (budget +8),
`scene_sync.gd` 50 (budget ~30), `scene_sync_cases.gd` 529 (budget ~280–380),
R1 86 added Rust lines (budget ~50), docs +34 (budget +20–30). The case file is
over because each case keeps exact assertions and both directions plus its
controls; no contract check was dropped to fit.

## By-hand run

Pending, and it is the user's: `Godot4 --path apps/client`, play to the end; a
proxy is grey exactly where the HUD shows `(dead)`, and both proxies remain.

## Files

- `apps/client/scripts/SceneSync.gd` (rewritten)
- `apps/client/tests/scene_sync_cases.gd` (new)
- `apps/client/tests/scene_sync.gd` (new)
- `apps/client/tests/smoke.gd` (cases hook, exit 6)
- `crates/crpg-godot/src/core/client/tests.rs` (R1 only)
- `docs/architecture/crpg-godot.md`
- `crates/crpg-godot/AGENTS.md`

No golden or replay file changed; no S-set file was touched.

## Review fix round (session 35)

The Claude review (Sonnet 5.5) planted 36 mutants in `SceneSync.gd`: 31 killed, 5 survived (no `despawn.sort()`, no `refresh.sort()`, `dead == 1`, `proxy_count` counting every child, a new material on every dead sync). Attempt 2 tightened `scene_sync_cases.gd` (`da50bac`) and hung in its final gate run; the manager committed the edit, `verify_task.py` passed 11/11, `scene_sync.gd` and `smoke.gd` exit 0, and all five mutants now exit 6. The by-hand windowed run is still the user's.

## Fix round 2 — PASS sentinel

A Godot 4.7.2 probe showed a GDScript runtime error inside a `-> String`
function returns `""`, not `null`, so the empty-string success sentinel made a
crashing case or helper read as a pass. The case file now reports success with
the explicit `"PASS"` token; the runner and smoke hook fail (exit 6) on
anything that is not the `String` `"PASS"`.

### Red checks (each planted, run, expect 6, then restored)

| Plant | Command | Exit | Output |
|---|---|---|---|
| R-a: `var n = null; n.foo()` as the first line of `_case_s2` | `scene_sync.gd` | 6 | `scene_sync.gd FAIL: S2: runtime error` |
| R-b: `var n = null; n.foo()` as the first line of `_expect_pos` | `scene_sync.gd` | 6 | `scene_sync.gd FAIL: S1: runtime error` |
| R-c: `run` returns `""` at the end | `scene_sync.gd` | 6 | `scene_sync.gd FAIL: ` (empty result) |
| R-d: R-a planted | `smoke.gd` | 6 | `smoke.gd FAIL: scene sync S2: runtime error` |
| — unplanted | `scene_sync.gd` | 0 | `scene_sync.gd OK` |

### Definition of done

Run from Git Bash at the worktree root, `CARGO_TARGET_DIR` and
`GDRUST_MAIN_EXTENSION` unset. Every command and its exit code:

| Command | Exit |
|---|---|
| `cargo fmt --all` | 0 |
| `cargo clippy -p crpg-godot --all-targets --locked -j 2 -- -D warnings` | 0 |
| `cargo test -p crpg-godot --locked -j 2` | 0 (45 unit, 2 bridge, 2 source-rules) |
| `python tools/lint/deps.py` | 0 |
| `python tools/lint/determinism.py` | 0 |
| `python -m unittest discover -s tools/lint -p "test_*.py"` | 0 (81 tests) |
| `cargo clippy -p crpg-godot --features engine --all-targets --locked -j 2 -- -D warnings` | 0 |
| `cargo build -p crpg-godot --features engine --locked -j 2` | 0 |
| `bash tools/preflight.sh --crate crpg-godot` | 0 (see reading below) |
| `Godot4 --headless --path apps/client --import` | 0 |
| `Godot4 --headless --path apps/client -s res://tests/scene_sync.gd` | 0 (`scene_sync.gd OK`) |
| `Godot4 --headless --path apps/client -s res://tests/load.gd` | 0 (`load.gd OK`) |
| `CRPG_CAMPAIGN=/no/such/dir ... -s res://tests/load.gd` | 3 (`open = 20`) |
| `Godot4 --headless --path apps/client -s res://tests/smoke.gd` | 0 (`smoke.gd OK`) |
| `CRPG_CAMPAIGN=/no/such/dir ... -s res://tests/smoke.gd` | 3 (`open = 20`) |
| `grep -n 'return ""' apps/client/tests/scene_sync_cases.gd` | 1 (no match) |
| `git diff --check` (after removing the `*.uid` files) | 0 |

### Reading

`bash tools/preflight.sh` calls `python3`, which on this slot resolves to the
Windows Store alias (`python` is the real Python 3.12). I ran preflight with a
`python3` shim (`exec python "$@"`) on PATH; it passed all gates, including
`cargo deny check`. The crate's own AGENTS.md gates use `python` and passed
directly. This is an environment fix, not a gate change.

The by-hand windowed run stays pending (the user's): `Godot4 --path
apps/client`, play to the end; a proxy is grey exactly where the HUD shows
`(dead)`, and both proxies remain.

## Agent log

- 2026-11-03 (UTC) · opencode-go/deepseek-v4.1-flash + T048 · Recorded the red-check exit 6, every Definition-of-done exit code, the four implementation readings and the pending by-hand windowed run, so the next task inherits the measured baseline rather than re-deriving it.
- 2026-11-03 (UTC) · claude-code/opus-5.5 (manager) + T048 review fix · Recorded the review's mutant result and the attempt-2 fix verification, since the worker hung before writing it.
- 2026-11-07 (UTC) · opencode-go/deepseek-v4-pro + T048-fix · Recorded fix round 2: the explicit `"PASS"` success sentinel with its three red checks, every Definition-of-done exit code, and the preflight `python3`-shim reading, so the sentinel change is audited end to end.
