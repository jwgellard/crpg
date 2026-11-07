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

## Fix round (T051-fix)

Two first-pass reviews of `dc54dff` listed seven contract items with no case that
would catch a break. Each lead was checked against the contract and the cases.
Five gaps are real and are now covered by tightened/new cases, each proven by a
planted bug that turns `hud.gd` red (exit 6) and was then undone. Two are not
real gaps.

| # | Lead | Verdict | Case | Red check (planted bug) |
|---|---|---|---|---|
| 1 | B2 — `sync(null)` leaves Waiting/Status/End and `_logged_revision` | real | H17 tightened | exit 6 `H17: null sync` (planted `_logged_revision = 5`) |
| 2 | B5 — press an option with `main == null` | real | H20 new | exit 6 `H20: log: got ["NULL"]` (planted notice on null) |
| 3 | B8 — `sync` never touches Status | real | H18 tightened | exit 6 `H18: Status visible: got false` (planted `sync` hides Status) |
| 4 | D7 — code 30 with `main.session == null` uses `NOT_OPEN` | real | H6 tightened | exit 6 `H6: 30 null last line` (planted dropped null guard) |
| 5 | D13 — an undisclosed row is the only thing keeping a side alive | real | H15 tightened | exit 6 `H15: dead [1, 1, 0]` (planted `_winner` excludes undisclosed) |
| 6 | B12 — unequal parallel columns | not real | — | B12 is a negative "not defended" precondition with no observable behaviour to assert; the equal-length invariant it relies on is already pinned by H3/H14/H15. |
| 7 | D17 — runner treats a `""`/`null` crash as failure | not real | — | Sentinel is structural: `run` returns "PASS" only when every case returned "ok", maps `""`/`null` to "runtime error", and `hud.gd` exits 6; re-verified with a crash plant (exit 6 `H2: runtime error`). |

Readings:

- Lead 2's "dereferences null" form is a `-> void` signal handler dereferencing
  `null`, so it returns silently (the failed `choose` reads as `0` and hits the
  early return) and is unobservable from GDScript; the catchable form is "does
  not no-op" (an observable side effect), which H20 covers. Confirmed with a
  deref plant that still exited 0.
- H20 (press with `main == null`) reuses the contract's next-free case number;
  smoke.gd's real-session H20 is a separate check in a separate file.

### Agent log

- 2026-11-07 (UTC) · opencode-go/deepseek-v4-pro + T051-fix · Closed five review gaps (B2/H17, B5/H20, B8/H18, D7/H6, D13/H15) with tightened or new cases, each proven by a planted bug that made `hud.gd` exit 6 and was undone; judged B12 and D17 not real gaps and recorded why.
