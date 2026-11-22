# T156a mutant run

Tree: `task/T156a-i1` at `004c49f`. Contract: `tasks/T156.md` (Parts, A-1..A-6,
Readings, Tests A1-A9). Scope: new and changed non-test lines of
`crates/crpg-godot/src/core/mod.rs` and `crates/crpg-godot/src/core/authority/v4.rs`
(`git diff c166c68 004c49f`).

Command per mutant: `CARGO_INCREMENTAL=0 cargo test -p crpg-godot --locked -j 2 --lib`
(timeout 600 s). Baseline (unmutated): 120 passed, 0 failed. Files are LF.
Driver `run_mutants.py` (exact-text, anchor must occur once, originals restored
in `finally`); rows in `results.jsonl`; raw output in `run.log`.

**35 mutants: 23 killed, 12 survived, 0 compile-error, 0 anchor-error.**
Of the 12 survivors: 5 are real test gaps (M17, M18, M19, M23, M26), 7 are
equivalent or unreachable (M03, M07, M09, M11, M32, M33, M34).

## Table

| id | item | edit | result (first killers) |
|---|---|---|---|
| M01 | A-1 | `open_v4_ai_with` passes `NpcDriver::Peer` | killed A1 A2 A3 A7 |
| M02 | A-1/A-6 | `open_v4_with` passes `ServerAi` | killed A1 A2 A7 |
| M03 | A-1/A-6 | `open_v4_rt_with` passes `ServerAi` | SURVIVED (equivalent) |
| M04 | A-1 | `open_v4_ai_with` uses `RealTime` | killed A1 A2 A3 A5 A6 A7 A8 |
| M05 | A-1 | `open_v4_mode` forwards `Peer`, dropping `driver` | killed A1 A2 A3 A7 |
| M06 | A-1 | ServerAi branch yields `npc_actors = None` | killed A1 A2 A3 A7 |
| M07 | A-1 | Peer branch also clones `npc_actors` | SURVIVED (equivalent) |
| M08 | A-1 | rows' side string replaced by `""` | killed A1 A2 A3 A7 |
| M09 | A-1/A-2 | `prefix.bound` iterated `.rev()` | SURVIVED (equivalent) |
| M10 | A-1 | `ServerAiHook::new(.., Vec::new())` | killed A1 A2 A3 A7 |
| M11 | A-1/A-2 | `ai_sides` `Err` arm swallowed (empty `Sides`) | SURVIVED (unreachable) |
| M12 | A-2 | names iterated `.rev()` before numbering | killed A4 |
| M13 | A-2 | `SideId(index + 1)` | killed A4 |
| M14 | A-2 | every side `SideId(0)` | killed A4 A1 A2 A3 A7 |
| M15 | A-2 | duplicate code `BadArgument` | killed A4 |
| M16 | A-2 | duplicate detail `String::new()` | killed A4 |
| M17 | A-3 | `AI_STEP_DELAY_MS` 600 to 300 | SURVIVED (real gap) |
| M18 | A-3 | not-AI-turn no longer clears `due` | SURVIVED (real gap) |
| M19 | A-3 | `saturating_add` to `wrapping_add` | SURVIVED (real gap) |
| M20 | A-3 | arming step returns `true` | killed A9 A1 A2 A3 A7 |
| M21 | A-3 | `now_ms >= due` to `>` | killed A9 A1 A3 |
| M22 | A-3 | firing does not disarm | killed A9 |
| M23 | A-3 | firing re-arms at `now + D` instead of disarming | SURVIVED (real gap, low) |
| M24 | A-4(1) | `is_closed` check removed | killed A6 |
| M25 | A-4(2) | delivery acknowledgement removed | killed A5 |
| M26 | A-4(2) | acknowledges `entries[0]` not `last` | SURVIVED (real gap) |
| M27 | A-4(3) | `ai_turn` inverted | killed A1 A2 A3 A7 |
| M28 | A-4(3) | `ai_turn = true` (gate removed) | killed A3 |
| M29 | A-4(4) | pacer gets `now = 0` | killed A1 A2 A3 A7 |
| M30 | A-4(4) | `drive_ai(.., &[], ..)` | killed A1 A2 A3 A7 |
| M31 | A-4(4) | budget `decisions_per_turn: 0` | killed A1 A2 A3 A7 |
| M32 | A-4(4) | `drive_ai` called twice per firing | SURVIVED (equivalent on `combat_basic`) |
| M33 | A-4(4) | `drive_ai` error swallowed (`let _ =`) | SURVIVED (unreachable) |
| M34 | A-4 | drain moved after pacer/`drive_ai` | SURVIVED (equivalent) |
| M35 | A-4 | `into_hook` passes `0 * now_ms` | killed A1 A2 A3 A7 |

## Survivors, exact edit and class

Real gaps (a test is missing; no source defect found):

- **M17 (A-3, D78b)**: `pub(crate) const AI_STEP_DELAY_MS: u64 = 600;` to `= 300;`.
  A1/A3/A9 take `D` from the constant, A2 steps by 600 (which still works at 300),
  A7 has no lower bound. Nothing in the crate pins 600 (the Godot `enemy_ai.gd` E3
  pins `D - 100` in T156b). Suggest `assert_eq!(AI_STEP_DELAY_MS, 600)` in A9, or
  an A1 assertion that nothing acts at `settle(599)`-equivalent against the literal.
- **M18 (A-3)**: delete `self.due = None;` in the `!ai_turn` arm. A9's
  `step(2D+1, false)` runs right after a firing, when `due` is already `None`,
  so it never disarms an armed pacer. The contract's own A9 sequence has this
  flaw. Fix: arm, then `step(t, false)`, then a true step before the old due must
  re-arm (for example `step(0,true)`, `step(1,false)`, `step(D,true)` must be false).
  A1/A3 do not help: the Strike ends the turn, so no run arms and then loses the turn.
- **M19 (A-3)**: `saturating_add` to `wrapping_add`. A9's saturation case
  (`step(MAX-1)` false, `step(MAX)` true) also passes with wrap (`due` = 598).
  Fix: a second `step(u64::MAX - 1, true)` must be false.
- **M23 (A-3)**: after a firing, `self.due = Some(now_ms.saturating_add(AI_STEP_DELAY_MS))`
  instead of `None`. A9 re-steps at the same `now`, so both rules arm at the same
  due; A1/A3 end the turn on the Strike. Fix: after a firing at `D`, `step(D + 50, true)`
  then `step(2D, true)` must be false (due `2D + 50`).
- **M26 (A-4(2))**: `last.ordinal` to `entries[0].ordinal`. A5's NPC delivery holds
  one entry in the drained cycle, so first and last coincide. Fix: a drain over two or
  more entries (for example let two cycles pass without the hook, then run it).

Equivalent or unreachable:

- **M03**: `open_v4_rt_with` with `ServerAi`. Real-time NPC legal options are empty
  (T050 Reading 1), so `ServerAiHook` never fires and drains exactly as `NpcHook`;
  no observable difference. The A-6 RT pin cannot see it.
- **M07**: Peer branch clones `npc_actors`; the `(_, _)` arm ignores it.
- **M09**: `Sides::new` sorts by `EntityId` (`crates/crpg-ai/src/sides.rs:55-56`), so
  row order is not observable.
- **M11**: `Err(fault)` arm of `ai_sides` in `open_v4_guarded`: bound entities are
  distinct, so it cannot run (contract says so); no seam reaches it. A-1's "25, no runner"
  for it has no test, only A4 for the function.
- **M32**: `drive_ai` twice per firing. On `combat_basic` the Strike has `ends_turn`,
  so the second call runs on the Hero's turn (actors = Goblin) and does nothing. "One
  decision per call" (A-4(4)) would need a fixture ability that does not end the turn.
- **M33**: `?` to `let _ =` on `drive_ai`. After the `is_closed` check no
  `drive_ai` error is reachable from a test.
- **M34**: drain after `drive_ai` instead of before the gate. The only difference is
  which cycle an AI-published delta is drained in; the delivery is not read by any test
  at that granularity.

## The implementer's two notes

1. **A1/A3 timelines.** Corrected A1/A3 still pin the pacing relative to `D`: M20
   (arm fires at once), M21 (`>`), M29 (`now = 0`), M35 and the `D - 50` / `D` window
   in A1 and A3 are killed. They do **not** pin the constant (M17) and no longer
   exercise the re-arm/disarm rules, because the Strike ends the turn: that is why
   M18 and M23 live. A9 carries the arm/fire rules but, as written in the contract,
   not the disarm-while-armed rule.
2. **A6 and `is_closed`.** Not equivalent: M24 (check removed) is killed by A6
   (the drain path alone does not give the exact `HOST_CLOSED: Closed at host/admission` end A6 asserts; cause not investigated further).
   No test-side change needed.

## Tests against contract values

A1-A9 assert the contract's values: A1 (actor 2, target 1, "Strike", `X() == 0`,
peer control `X() == 1` at 650), A2, A3, A4 (ids 2/1/1/0, hostile pairs, empty,
duplicate text), A5, A6, A7, A8, A9 (the contract's exact sequence). The only
deviations are the implementer's, on the `ends_turn` fact: A1 (no second action, the
Hero's `TurnStarted` in the same cycle, no turn change through `2D + 100`), A3
(`X() == 1`, not 2). Both are consistent with the code. No contract item is
silently dropped. Reached by no mutant or test: the `Err(fault)` path of A-1 (25, no
runner), "calls nothing else" (A-4, no `ingest`/NPC sequence), and one-decision-per-call.

## Verdict

**fix** (one round, tests only): add the five missing assertions (M17, M18, M19, M23, M26)
to `ai.rs` (A9 for M17/M18/M19/M23, A5 for M26). No source defect found. `git status --short crates`
and `git diff -- crates` were empty after the last batch.

## Agent log
- 2026-11-22 (UTC) · claude-code/claude-sonnet-5-5 + T156a mutant run · 35-row mutant table over the T156a source lines, results and survivors recorded for the manager; no source or test edited.
