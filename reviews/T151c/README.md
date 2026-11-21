# T151c mutant run

Branch `task/T151c-i1` at `ef2ad53`. Target: the new non-test lines of
`crates/crpg-godot/src/shell.rs` (`git diff c953a39 ef2ad53`). Gate per mutant:
`cargo build -p crpg-godot --features engine --locked -j 2` (CARGO_INCREMENTAL=0),
then `Godot4 --headless --path apps/client -s res://tests/movement.gd` from Git
Bash; non-zero exit = killed. Baseline (unmutated): exit 0, `movement.gd OK`.
Driver `run_mutants.py`, records `results.jsonl`, log `run.log`.

**Result: 11 killed / 20 (0 compile errors), 9 survived.**

| id | line / change | B-item | result |
|----|---------------|--------|--------|
| C01 | `open_v4`: calls `core.open` | B-C1 | killed (6, M1 position_known) |
| C02 | `open_v4`: `opened_at` assignment removed | B-C1 | **survived** |
| C03 | `open_v4`: encounter_id/player_side swapped | B-C1 | killed (3, M1 open_v4) |
| C04 | `open_v4`: player_side -> "" | B-C1 | killed (6) |
| C05 | `declare_move`: revision -> 0 | B-C1 | killed (3, M2 control) |
| C06 | `declare_move`: x and z swapped | B-C1 | **survived** |
| C07 | `declare_move`: y and z swapped | B-C1 | killed (6, M2 points) |
| C08 | `move_to`: actor -> 2 | B-C1 | killed (3, M3 control) |
| C09 | `move_to`: calls `core.halt(actor)` | B-C1 | **survived** |
| C10 | `move_to`: (x,y,z) -> (0,0,0) | B-C1 | **survived** |
| C11 | `halt`: calls `core.move_to(actor,0,0,0)` | B-C1 | **survived** |
| C12 | `halt`: returns constant 0 (core call kept) | B-C1 | **survived** |
| C13 | `halt`: actor -> 2 | B-C1 | **survived** |
| C14 | `positions`: empty array | B-C2 | killed (6, M1) |
| C15 | `positions`: elements reversed | B-C2 | killed (6, M2 tail) |
| C16 | `position_known`: copies `moving()` | B-C2 | killed (6, M1) |
| C17 | `moving`: all zeros, right length | B-C2 | **survived** |
| C18 | `move_receipts`: `.take(0)` | B-C2 | killed (4, M3 timeout) |
| C19 | `move_receipts`: `.rev()` | B-C2 | **survived** |
| C20 | `move_receipts`: copies `receipts()` | B-C2 | killed (4, M3 timeout) |

## Survivors (exact edit, B-item, class)

All nine are real test gaps in `apps/client/tests/movement.gd`; none is
equivalent. No `shell.rs` defect was found (the unmutated code is right).

- **C02** B-C1: delete `self.opened_at = Some(Instant::now());` in `open_v4`.
  Then `poll()` passes `now_ms = 0` forever. `poll_v4_guarded` feeds `now_ms`
  to `ClientV4::step` (`client_v4.rs:94-142`, the pool timers), so the edit is
  observable in principle, but no M1-M5 case runs long enough to reach a timer.
  The contract line "sets `opened_at` exactly as `open` does" has no engine
  test; K32 and the unit suite cannot see the shell.
- **C06** B-C1: `declare_move(revision, z, y, x)`. M2 uses the symmetric
  destination `(32768, 0, 32768)`, so an x/z swap is invisible (C07 y/z is
  killed). Use an asymmetric destination (e.g. x != z).
- **C09** B-C1: `move_to` calls `core.halt(actor)`. M3 asserts only seq 1,
  actor 2, "rejected", code 22, start_tick 0; a halt receipt of an idle actor
  matches all of them.
- **C10** B-C1: `move_to` passes `(0,0,0)`. M3 itself calls `move_to(2, 0, 0, 0)`,
  so the destination is unobservable. (The coordinate order is checked only
  through `declare_move`, not `move_to`.)
- **C11** B-C1: `halt` calls `core.move_to(actor, 0, 0, 0)`. Same receipt
  shape as C09 (M4 asserts seq 2, actor 2, rejected, 22).
- **C12** B-C1: `halt` returns constant 0. M4 has no error control
  (`halt(-1) == 3` is absent, unlike M3's `move_to(-1,...) == 3`).
- **C13** B-C1: `halt` passes constant 2. Same cause: M4 only ever halts 2.
- **C17** B-C2: `moving` returns zeros. M1 asserts `[0,0]` before any move and
  no case ever observes a `1`.
- **C19** B-C2: `move_receipts` iterated in reverse. Every view has exactly one
  receipt, so order ("oldest first") is never tested.

## M1-M5 against the contract

Every value M1-M5 asserts matches `tasks/T151.md` (M1 columns and empty
receipts; M2 control 4 then 0, `moved` entity 2, points
`[0,0,0,32768,0,32768]`, positions tail; M3 control 3, receipt seq 1, actor 2,
"rejected", code 22, start_tick 0; M4 seq 2; M5 size 6, `[0,0]`, code 3). The
test adds harmless waits (options present before M2; a later revision for M4).
No assertion contradicts the contract.

## Contract items no test reaches

- B-C1 `opened_at` set by `open_v4` (C02).
- `halt`'s error code path (no negative control) and `declare_move`'s codes
  other than 4/0.
- B-C2 `moving` set to 1, the `position_known` 0/1 mix, and multi-receipt order.
- B-C3 is K32's (untouched by every mutant; no mutant adds `unsafe`).

## Verdict

fix: one round on `movement.gd` only (asymmetric declare_move destination,
non-zero move_to destination, `halt(-1) == 3`, halt vs move_to told apart,
a `moving == 1` observation, two receipts in one view). C02 needs either a
long-running case or an accepted gap with a note.

## Fix round A

`apps/client/tests/movement.gd` gains M6 and M7 (M1-M5 unchanged); `shell.rs`
and `core/` are untouched. Re-run of the nine survivors (`run.log`,
`results.jsonl` updated): **6 killed, 3 accepted gaps.** The unmutated tree
passes `movement.gd` (twice) and `smoke.gd`.

- **M6** (fresh session, the turn pool unspent): `declare_move(rev, 32768, 0,
  16384)` then the `moved` event's points are `[0,0,0, 32768,0,16384]` and the
  positions tail is `[32768,0,16384]`. Reasoned from T037 K2
  (`tasks/T037.md:1300-1306`): straight route from the start, destination
  within one step (length 36,636 < 65,536).
- **M7**: `move_to(2, 536870913, 0, 0) == 3` (`COORD_LIMIT_RAW + 1`, B-B8 (1),
  `tasks/T151.md:670-676`, B11 `:923`), `halt(-1) == 3`, `halt(0) == 3` (B-B8
  (1)); then `move_to(2, ..)` and `halt(2)` back to back: one view whose
  `move_receipts()` are `[{seq 1, actor 2, "superseded", code 0, start_tick 0},
  {seq 2, actor 2, "rejected", code 22, start_tick 0}]` (T037 D-6/D-7
  `tasks/T037.md:770-781`, D-3 release in seq order `:743-752`, A4 `:778-780`
  for "superseded"/0/0, B-C2 oldest first).

| id | result | by |
|----|--------|----|
| C02 | **accepted gap** | see below |
| C06 | killed (6) | M6 points `[0,0,0,16384,0,32768]` |
| C09 | killed (3) | M7 `move_to(x out of range) == 3` (halt ignores the coordinates) |
| C10 | killed (3) | M7 `move_to(x out of range) == 3` (the constant 0 is in range) |
| C11 | **accepted gap** | see below |
| C12 | killed (3) | M7 `halt(-1) == 3` |
| C13 | killed (3) | M7 `halt(-1) == 3` |
| C17 | **accepted gap** | see below |
| C19 | killed (6) | M7 receipt order (mutant gives seq 2 first) |

Accepted gaps (no engine-gate case with a contract-stated value):
- **C02** (`opened_at` not set): the edit leaves `now_ms` at 0, which only the
  client pool's timers read (`client_v4.rs:94-142`). Reaching one needs a stalled
  transfer and a long wait, and the contract states no value for it; a sleep to
  reach a timer was ruled out by the brief. K32 and the unit suite cannot see the
  shell either.
- **C11** (`halt` calls `core.move_to(actor,0,0,0)`): the engine gate runs a
  turn-based encounter (`core/mod.rs:584` `start_encounter`, never `_in_mode`),
  where every `MoveTo` and `Halt` of a combatant is `Rejected(WrongMode)` 22
  (T037 D-7 (1), `:777`) or `NotAuthorized` 17 for a non-controlled actor;
  admission does not look at the body (D-2, `:719-741`). The two intents yield
  identical receipts and returns, and `halt`'s only argument check (`actor`) is
  the same as `move_to`'s. A real-time encounter (T037e K3) cannot be started
  from data through the shell.
- **C17** (`moving` all zeros): `moving` is `1` only for a real-time sample
  (T037e K3, tick 1 `true`, `tasks/T037.md:1308-1313`); a turn-based encounter,
  the only kind the shell starts, always reports `[0,0]` (B4, `tasks/T151.md:864`),
  and `moving`'s element copy is the same code as `position_known`'s (C16 is
  killed). The host side of `moving == 1` is T037e K3's.

## Agent log
- 2026-11-21 (UTC) · claude-code/Sonnet 5.5 (reviewer) + T151c mutant run · Ran 20 single-point mutants of shell.rs against movement.gd through the engine gates: 11 killed, 9 survived, all nine real test gaps.
- 2026-11-21 (UTC) · claude-code/Sonnet 5.5 (implementer) + T151c fix round A · Added M6/M7 to movement.gd, killing six of the nine survivors, and recorded C02, C11 and C17 as accepted gaps with reasons.
