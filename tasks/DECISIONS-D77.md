# Decision D77 — movement in the Godot client (T050, T049, T026)

Status: **open.** Contracts `tasks/T050.md` (T050a, T050b), `tasks/T049.md`
(T049a) and the revision at the end of `tasks/T026.md`, all written for the
recommended answers below (architect, T026c).

## What you would see
Launch the walk scene, click the floor, and the selected Hero walks there at
walking pace; click the other Hero to select it and send it somewhere else
while the first is still walking. Both are drawn smoothly. The server running
in the background decides every step, exactly as in multiplayer.

## Questions

**Q1 (D77a) — Should the walk demo be a real-time scene?** Today every
fight the game opens is turn-based, and the server refuses free walking in a
turn-based fight. The engine can already start a fight in real-time mode,
where walking is free.
- **(a) Real time (recommended).** Clicking walks a Hero continuously, and you
  can steer both Heroes at once. The catch: in this build nobody attacks in
  real time (the server offers combat choices only on someone's turn), so the
  Goblin just stands there and the combat panel stays empty. Real-time combat
  is a later server task.
- **(b) Turn-based steps.** On your turn a click moves your Hero along a
  route, spending its one action (one metre per action in the test rules);
  nothing else moves. Smooth movement of other characters cannot be shown.

**Q2 (D77b) — May the demo add its own campaign, `campaigns/demo/walk`?**
The test campaigns may not contain movement rules (a test enforces it), and
the test arena is one metre square. The new campaign is a copy of
`combat_basic` with walking rules (2 metres a second), real time allowed, a
16 × 12 metre floor, and a second Hero.
- **(a) Yes (recommended):** a checked-in campaign anyone can launch.
- **(b) No:** build it at test time only, as the existing movement test does;
  then there is no demo you can open.

**Q3 (D77c) — Approve three new functions between the game and Rust?** This
boundary is on the list you review yourself. The additions are: open a
real-time session; "which characters may I move" (a list of ids); and "where
to draw every character this frame". The bridge design document planned a
different shape for drawing (`previous_positions()`, where the game script
would do the smoothing); the contract puts smoothing in Rust instead, because
there it is exact and tested on every build.
- **(a) Approve (recommended).**
- **(b) Keep the planned shape:** the smoothing is written in game script,
  which only the local Godot runs can test.

**Q4 (D77d) — How do you launch it?**
- **(a) A new scene, launched by one command (recommended now):**
  `Godot4 --path apps/client res://scenes/Walk.tscn`. The combat demo stays
  what opens by default; CI and the downloadable build are untouched.
- **(b) Make the walk scene the default** when the client opens. One line;
  the combat demo then needs a command instead.
- **(c) Also put the walk scene in CI and in the downloadable build.** This
  edits the CI workflow (on your review list), so it is its own small task.
  Recommended after you have played it locally.

**Q5 (D77e) — Build own-movement prediction (T026) now?** Prediction moves
your own character before the server confirms, to hide network delay. In the
single-player game the server runs in the same program, so there is no
network delay to hide: after the smoothing task, the lag from click to motion
is at most about 150 ms for every character, the same as everything else
on screen.
- **(a) Defer it (recommended)** until the game can join a server over the
  network, which is where it shows. `tasks/T026.md` now lists what it will
  need first.
- **(b) Build it now** for single-player. It needs a new dependency from the
  game bridge to the navigation library (your approval), a copy of the floor
  map in the client, and a second movement path with corrections, for no
  visible change.

## Agent log
- 2026-11-21 (UTC) · claude-code/opus-5.5 (architect) + T026c · Drafted D77 with the five choices only the user can make for the walk demo (real-time scene, a demo campaign, the three bridge functions, how to launch it, and whether prediction waits), each with the contract's recommendation.
