# Decision D79 — walk, then fight (T157)

Status: **open, waiting for the user.** Contract `tasks/T157.md` (architect,
attempt 1) is written for every recommended option (a); each question says
what changes otherwise.

## What you would see
Start the new scene (`Godot4 --path apps/client res://scenes/Adventure.tscn`).
One hero stands in the room and a goblin waits at the far side. Left-click the
floor to walk, right-click to stop, exactly as in today's walk scene; the
goblin does not move. Click the goblin: within half a second the fight starts
where everyone stands. The goblin, played by the same server AI as before,
walks up to 6 m toward you and strikes if it reaches. On your turn, click the
floor to step (6 m of movement per turn; a click too far is refused and the
log says so), click the goblin to strike it once you are within 1.5 m, or
press "End turn". Health bars, damage numbers and hit flashes work as in the
combat demo. When the goblin falls, click the floor and you walk again.

Two findings shape the questions. First, the engine ends a fight only when
fewer than two fighters are alive: it has no notion of "sides" yet, so with
two heroes the fight would go on after the goblin dies. Second, when a
turn-based fight ends, the engine's movement rules still treat the survivors
as "in a turn-based fight" and refuse to let them walk, and nothing in the
game's background server can close the finished fight.

## Questions

**Q1 (D79a) — How does the fight start?** "Turn-based mode" is the fight
mode where everyone acts in turn; "real time" is the walking mode. Switching
is a request to the server, which applies it at the next round boundary
(at most half a second with the demo's settings).
- **(a) You click the enemy while walking (recommended).** The click sends
  "switch to turn-based" to the server. Nothing in the server changes, and
  the game script still decides no rule: it does not measure distance or
  check who is an enemy; the server decides whether the request is allowed.
- **(b) The server starts the fight when a hero comes near the enemy.** The
  enemy "notices" you at some distance. This is a new server rule (it would
  also run on the dedicated server), so it adds a server task and a design
  question about the distance and who triggers it. The click-to-engage path
  would be removed.
- **(c) Both.** (b)'s extra task, and the click stays as a way to start
  early.

**Q2 (D79b) — How does the fight end, so that you walk again?**
- **(a) One hero for now; a finished fight lets its survivors walk
  (recommended).** The demo has one hero and one goblin, so the fight ends
  when either falls. A small engine change (in the simulation and in the
  server's movement check, one condition each) says that a turn-based fight
  with nobody's turn left holds no one, which is already how a finished
  real-time fight behaves. This amends an accepted design record (ADR-0036)
  and changes the expected answer in two existing tests that pinned the old
  behaviour; that is the rule changing by your decision, not a test being
  weakened, so it needs your yes. Parties of two or more wait for "sides"
  (a later task).
- **(b) Two heroes, no engine change; you leave the fight yourself.** After
  the goblin dies the fight goes on, so you press a "Leave the fight" key,
  which takes effect when the round ends (each remaining hero ends its
  turn). Meanwhile the heroes' options include striking each other, and the
  same key would let the party walk away from a living goblin.
- **(c) Teach the engine sides first.** The fight ends when one side is
  left, so parties work properly. This is a larger change to the simulation,
  the campaign data and another accepted design record (ADR-0029); the demo
  waits for it.

**Q3 (D79c) — Should the screen show how much movement is left?**
- **(a) Not in this task (recommended).** The allowance is enforced by the
  server; a click beyond it is refused and the log shows "Rejected: no longer
  legal (code 18)". A "Move: 4 of 6 m" readout and a reach ring become the
  first follow-up.
- **(b) Yes, now.** The server must disclose the hero's own movement pool to
  the game, the bridge gains one more function the game can call (one more
  item on your review list) and the HUD a line. About one more part's work.

## Not questions (decided in the contract)
- Walking happens inside the area's encounter in real-time mode, as today's
  walk scene does; the enemy simply does not act in real time. Real
  exploration outside any fight, and starting or closing fights during play,
  are a later server task.
- The demo uses a new campaign, `campaigns/demo/adventure` (the walk
  campaign's room, minus the second hero, with a separate 6 m movement
  allowance per turn, a 1.5 m reach for Strike and a sturdier hero); the
  existing walk campaign and every test fixture stay as they are.
- `Adventure.tscn` is a new scene that reuses the walk scene's script;
  `Main.tscn` stays the scene that opens by default and that the automatic
  Windows check plays. Making the new scene the default is a later workflow
  task (on your review list).
- The two new bridge functions (`open_v4_walk`, `request_mode`) are on your
  review list: T157d waits for your review, as T156b did.

## Answers (the user, 2026-11-22)
- **D79a: (c) both.** A server rule starts the fight when a hero comes near the enemy, and clicking the enemy still starts it early.
- **D79b: (c) teach the engine sides first.** A fight ends when one side is left; parties work properly; ADR-0029 is amended. The demo waits for it.
- **D79c: (b) yes, now.** The server discloses the hero's own movement pool, the bridge gains the function, and the HUD shows what is left.

Status: **answered.** The contract goes back to the architect for revision 2.

## Agent log
- 2026-11-22 (UTC) · claude-code/opus-5.5 (architect) + T157 contract · Wrote the three user questions for walk-then-fight (how the fight starts, how it ends so you walk again, whether to show movement left), each with its recommendation, after finding that fights end only below two living fighters and that a finished turn-based fight freezes its survivors.
- 2026-11-22 (UTC) · claude-code/opus-5.5 (manager) + D79 answers · Recorded the user's answers (D79a c, D79b c, D79c b) so the contract revision builds on them.
