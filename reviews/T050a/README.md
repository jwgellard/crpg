# T050a mutant run

Branch `task/T050a-i1` at `469f7d2`. Driver `run_mutants.py`, one JSON line per
mutant in `results.jsonl`, cargo output summary in `run.log`. Command per row:
`cargo test -p crpg-godot --locked -j 2 --lib` (`CARGO_INCREMENTAL=0`). Baseline
(unmutated): 97 passed, 0 failed. Mutated file: `crates/crpg-godot/src/core/mod.rs`
only; restored after every row (`git status --short crates campaigns` empty).

Result: **28 mutants, 22 killed, 6 survived, 0 compile errors.**

| id | W-item | mutant | status | killed by |
|---|---|---|---|---|
| A01 | W-1 | open_v4_with: passes EncounterMode::RealTime instead of TurnBased | killed | fa_1_pause_rights_are_the_least_privilege_default, first_settle_equals_v2_and_replies_published, close_ends_the_loop, +20 more |
| A02 | W-1 | open_v4_rt_with: passes EncounterMode::TurnBased instead of RealTime | killed | rt_open_needs_round_ticks, rt_open_publishes_the_walk_world, rt_move_to_walks_to_the_destination |
| A03 | W-1 | open_v4_guarded: open_prefix gets constant TurnBased (mode ignored) | killed | rt_open_needs_round_ticks, rt_open_publishes_the_walk_world, rt_move_to_walks_to_the_destination |
| A04 | W-1/W-4 | open_prefix: start_encounter_in_mode gets constant RealTime (v2 open and TB v4 become real time) | killed | close_ends_the_loop, fa_6_declare_move_accepts_revision_zero, fa_1_pause_rights_are_the_least_privilege_default, +40 more |
| A05 | W-4 | open_on (v2): open_prefix gets RealTime instead of TurnBased | killed | controlled_and_render_positions_by_state, move_entry_precedence, fa_1_pause_rights_are_the_least_privilege_default, +25 more |
| A06 | W-1/W5 | open_v4_mode: already-open check hoisted before the bad-argument check (steps swapped) | survived |  |
| A07 | W-1/W5 | open_v4_mode: negative-seed test removed from the pre-guard check (falls through to open_prefix 'negative seed') | killed | rt_open_precedence |
| A08 | W-1/W5 | open_v4_mode: malformed-encounter-id test removed from the pre-guard check | killed | rt_open_precedence |
| A09 | W-1/W5 | open_v4_guarded: already-open return code AlreadyOpen -> BadArgument | killed | rt_open_precedence |
| A10 | W-1/W5 | open_v4_guarded: already-open check skipped for a live OpenV4 state (arm `State::OpenV4(_)` moved to the Closed arm) | killed | rt_open_precedence |
| A11 | W-1 | open_prefix: failed start_encounter_in_mode returns code NoEncounter instead of StartFailed | killed | rt_open_needs_round_ticks |
| A12 | W-1 | open_v4_rt: public wrapper calls open_v4_with (turn-based) instead of open_v4_rt_with | survived |  |
| A13 | W-2 | open_v4_guarded: controlled.sort_unstable() removed | survived |  |
| A14 | W-2 | open_v4_guarded: controlled sorted descending | killed | rt_open_publishes_the_walk_world, rt_open_precedence |
| A15 | W-2 | open_v4_guarded: controlled id offset by one after client::wide (client::wide(id.get()) + 1) | killed | rt_open_publishes_the_walk_world, rt_open_precedence, controlled_and_render_positions_by_state |
| A16 | W-2 | open_v4_guarded: only the first player actor is mapped (.take(1)) | killed | rt_open_publishes_the_walk_world, rt_open_precedence |
| A17 | W-2 | open_v4_guarded: replica id looked up through the npc peer instead of the player peer | survived |  |
| A18 | W-2 | open_v4_guarded: LiveV4 stores an empty controlled list | killed | rt_open_publishes_the_walk_world, rt_open_precedence, controlled_and_render_positions_by_state |
| A19 | W-2 | open_v4_guarded: real-time open clears controlled | killed | rt_open_publishes_the_walk_world, rt_open_precedence |
| A20 | W-2 | open_v4_guarded: turn-based open clears controlled | killed | controlled_and_render_positions_by_state, rt_open_publishes_the_walk_world |
| A21 | W-2 | controlled(): OpenV4 arm returns an empty Vec | killed | rt_open_precedence, rt_open_publishes_the_walk_world, controlled_and_render_positions_by_state |
| A22 | W-2 | controlled(): Open (v2) arm returns vec![0] | killed | controlled_and_render_positions_by_state |
| A23 | W-2 | controlled(): Fenced arm returns vec![0] | survived |  |
| A24 | W-2 | controlled(): Closed arm returns vec![0] | killed | controlled_and_render_positions_by_state, rt_open_needs_round_ticks |
| A25 | W-2 | open_v4_guarded: defensive lookup-failure arm returns BadArgument instead of StartFailed (arm unreachable by design) | survived |  |
| A26 | W-3 | render_positions(): returns an empty Vec | killed | controlled_and_render_positions_by_state, rt_open_publishes_the_walk_world, rt_move_to_walks_to_the_destination |
| A27 | W-3 | render_positions(): returns net_ids() instead of positions() | killed | controlled_and_render_positions_by_state, rt_open_publishes_the_walk_world, rt_move_to_walks_to_the_destination |
| A28 | W-3 | render_positions(): first three entries only (.take(3)) | killed | controlled_and_render_positions_by_state, rt_open_publishes_the_walk_world, rt_move_to_walks_to_the_destination |

## Survivors

| id | exact edit | W-item | class |
|---|---|---|---|
| A06 | `open_v4_mode`: insert `if !matches!(self.state, State::Closed) { return self.fail(AlreadyOpen, "session already open"); }` before the `seed < 0 / Ulid` bad-argument check | W-1, W5 | **real gap.** The contract's W5 "Fails if: the real-time open checks in another order" and the Errors row ("3 before the guard, then 2") are not backed: W5 only issues the bad-argument calls on a closed core (rt.rs:372-383), never with a session open, so hoisting the already-open check is invisible. A call `open_v4_rt_with(dir, "", "", -1, ..)` and `("nope")` on the live session, expecting 3, kills it. |
| A12 | `open_v4_rt` (public, SystemClock): body calls `open_v4_with` instead of `open_v4_rt_with` | W-1 | **real gap (low).** Every lib test goes through `open_v4_rt_with`; the public wrapper that the T050b shell will call has no lib test. An `open_v4_rt(walk, "", "", 1)` then `controlled()`/`view` check (or `last_error` on BASIC = 25) kills it. |
| A23 | `controlled()`: `State::Fenced(_)` arm returns `vec![0]` | W-2 | **real gap.** W-2 states an empty Vec in `Fenced`; W4 reaches Closed, Open (v2) and OpenV4 only. Fenced needs a fatal fault (existing bridge tests reach it); one `controlled().is_empty()` and `render_positions()` check there kills it. |
| A17 | `open_v4_guarded`: `host.replica_id(npc, *entity)` instead of `(player, *entity)` | W-2 | **weak, not equivalent in principle.** The NPC peer's map also holds the player's entities and both peers mint the same ids in the walk and BASIC fixtures, so the lists coincide. A fixture where the NPC peer's disclosure differs would kill it; low priority. |
| A13 | `open_v4_guarded`: `controlled.sort_unstable();` removed | W-2 | **equivalent for every campaign the tests can build**: `prefix.player_actors` is pushed in combatant (entity) order and ids are minted ascending in that order, so the list is already sorted. The sort is contract text with no observable effect today. |
| A25 | `open_v4_guarded`: defensive `Ok(None) | Err(_)` arm returns `BadArgument` instead of `StartFailed` | W-2 | **unreachable by design** (implementer's reading; `presentation_tables` refuses first). Agreed; not a finding. |

## W1-W5 against the contract

Each asserts the contract's values (net_ids, names, sides, position_known,
positions `[196608,0,327680,196608,0,458752,786432,0,393216]`, moving, active,
codes 25/17/22/3/2, the exact `last_error` strings, `controlled == [1,2]`/`[2]`,
`[0;6]`). Differences: W2 asserts `receipts.len() == 1` and W1/W5 add a TB
`controlled()` check (stricter, fine); W2 control A does not pin ticks (agreed
reading). W5 covers the order only on a closed core (A06). W4 does not reach
`Fenced` (A23). No mismatched value found.

## Contract items no test reaches

- Bad argument over already-open precedence (A06).
- `open_v4_rt` public wrapper (A12).
- `controlled()`/`render_positions()` in `Fenced` (A23).
- W-2 defensive lookup-failure arm (A25, by design).
- A mutant on "Closed-state cleanup on the failed real-time open": the failed
  open sets no state (`fail` only records the text), so there is no cleanup
  line to remove; W3's `runner().is_none()` and `controlled()` empty already
  cover the outcome.

## Verdict

**fix** (test-only, rt.rs, one round): extend W5 with bad-argument opens while a
session is open (A06), add a W4 `Fenced` check (A23) and a public `open_v4_rt`
smoke (A12). No source defect found: all 22 behaviour mutants of W-1..W-4 on
the mode wiring, controlled list, and `render_positions` die.

## Agent log
- 2026-11-22 (UTC) · claude-code/sonnet-5.5 + T050a mutant run · 28-row mutant table over the new `mod.rs` lines, run mechanically; recorded kills, survivors and the W1-W5 reading.
