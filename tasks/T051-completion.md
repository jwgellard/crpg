# T051 completion record

Branch `task/T051-i1`, base `mgr/session-35-batch`. GDScript and docs only: no Rust, FFI member, golden or fixture changed.

## Definition of done (Git Bash, Windows, Godot 4.7.2)

| Command | Exit |
|---|---|
| `cargo fmt --all` | 0 |
| `cargo clippy -p crpg-godot --all-targets --locked -j 2 -- -D warnings` | 0 |
| `cargo test -p crpg-godot --locked -j 2` | 0 (45 + 2 + 2 passed) |
| `python tools/lint/deps.py` / `determinism.py` | 0 / 0 |
| `python -m unittest discover -s tools/lint -p "test_*.py"` | 0 (81 tests) |
| `cargo clippy -p crpg-godot --features engine --all-targets --locked -j 2 -- -D warnings` | 0 |
| `cargo build -p crpg-godot --features engine --locked -j 2` | 0 |
| `Godot4 --headless --path apps/client --import` | 0 |
| `... -s res://tests/hud.gd` | 0 (`hud.gd OK`) |
| `... -s res://tests/scene_sync.gd` | 0 |
| `... -s res://tests/load.gd` | 0 |
| `CRPG_CAMPAIGN=/no/such/dir ... load.gd` | 3 |
| `... -s res://tests/smoke.gd` | 0 (`smoke.gd OK`, H20 included) |
| `CRPG_CAMPAIGN=/no/such/dir ... smoke.gd` | 3 |
| `git diff --check` | 0 |

The by-hand windowed play (`Godot4 --path apps/client`) is **pending**: it is the user's.

## Red checks

- Harness control, `hud.gd` against T047d's `CombatHud.gd` (before the rewrite): exit **6**, `hud.gd FAIL: H1: node Panel is missing`.
- Contract's deliberate bug (`_on_option` treats 30 as an error), after the rewrite: exit **6**, `H6: 30 lines: got ["ERROR: INTENT_REFUSED: link full"], want ["INTENT_REFUSED: link full"]`. Reverted.
- Planted runtime error in a case (`_h2` body calls a method on null): `hud.gd` exit **6**, `H2: runtime error`; the same under `smoke.gd` exits **7**, `hud H2: runtime error`. Reverted.
- Planted runtime error in a `-> String` helper (`_eq`): `hud.gd` exit **6**, `H1: runtime error`. Reverted.
- Planted runtime error in an `-> Array` helper (`_texts`): exit **6**, `H3: known: got [], want [...]`. Reverted.
- H20 mutants in `smoke.gd` (wrong opening-line prefix, wrong death wording, `round()` without `+1`): each exit **7** with a `smoke.gd FAIL: hud ...` detail. Reverted.

## Mutants run against `CombatHud.gd` (all exit 6 unless noted)

30 as error (H6), event round without `+1` (H8), target read from `actor` (H8), `>` to `>=` trim (equivalent: the trim keeps the last 200 either way; not killable), cap 199 (H12), slice off by one (H12, after adding a per-step size/first-line check; the first version of H12 missed it by parity), disable on press (H5), bad `bind` (H5), first-live-row winner / side set ignored (H15), applied receipts printed (H11), waiting ignores `ended` (H7), unknown-as-zero health (H3), dead hidden for undisclosed rows (H3, after adding the `? (dead)` stage), arithmetic id lookup, gate removed (H10), `clear` keeping the gate or the colour override (H18), `bbcode_enabled` true (H1), `_` not replaced in odd outcomes (H9, after adding that line), `placements()` read (crash, exit 6), options/health not rebuilt (H4/H3), null-session guard dropped (H6).

## Readings I had to make

- H7: the Waiting text is written on every sync, including when hidden by `ended()` (`Waiting for Hero...` for `A{option_labels [], ended true}`), per B6.
- H3/H9/H12 carry extra stages beyond the contract's lists (undisclosed-and-dead row, an underscore outcome, a per-step cap check) to kill mutants that the listed values let through.
- H1 also pins the panel's four anchors (0) and the HUD's and `Panel`/`Margin`'s child counts, which the "Built with" column implies.
- `FakeView.round()` shadows the GDScript built-in `round`; Godot accepts it.
- A null parent compares as `get_parent() == null`; `typeof` differs for a null object, so `_eq(..., null)` would fail.

## Agent log

- 2026-11-07 (UTC) · claude-code/sonnet-5.5 + T051 · Recorded the gate results, the red checks and the readings for the HUD, cases and smoke hooks; the by-hand play is left to the user.
