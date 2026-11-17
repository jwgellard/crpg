# Decision D66 — session 67: the run loop's two questions, the v2 session, and contract approvals

Status: **D66a–d answered by the user, 2026-11-17 (UTC), each as recommended.** Drafted by the manager in session 67.

## D66a — when the server falls behind (T154 Question 1) (answered)

**Answer (user): (a) slow down.**

The server runs the world 20 times a second, so each step has 50 ms. If a step takes longer (a big fight, a slow disk), the next one is late.

- (a) **Slow down:** run one step, skip the missed ones and carry on from now. The game runs slightly slower for a moment and never jumps forward.
- (b) **Catch up:** run every missed step back to back. Game time stays in line with the wall clock, but a stall is followed by a burst of movement, which can itself stall.

**Recommend (a).** T151 already rejected bursts, and nothing in the game depends on matching real time. Choosing (b) changes T154a's cadence rule and two of its tests.

## D66b — a runnable `crpg-server` program now, or in T040 (T154 Question 2) (answered)

**Answer (user): (a) leave the program to T040.**

T154 builds everything a dedicated server needs to run the loop, and proves a single-player save runs on it. A usable program also needs a certificate, an invitation file for players, a save to load and settings, all already planned as T040.

- (a) **Leave the program to T040**, which then only wires these pieces together.
- (b) **A bare-bones program now** (load a save, listen on a port, a test certificate), which T040 later replaces.

**Recommend (a).** (b) is throw-away work and needs choices (file formats, command-line flags) that T040 is meant to make.

## D66c — does the old (v2) single-player session move onto the loop too (T151 Question 5) (answered)

**Answer (user): (a) leave v2 as it is.**

The new (v4) session now works as you asked in D65a: the game only sends commands and shows what the server sends, and the server runs in the background. The old v2 session, which today's demo uses, still has the game drive the server each frame and play the enemy itself.

- (a) **Leave v2 as it is** until it is retired. Nothing in the current demo or its tests changes; new work and the movement demo use v4 only.
- (b) **Move v2 onto the loop too.** v2 would then also tick 20 times a second, which changes every message it sends (each carries the tick number), so its existing tests would need rewriting, which needs your approval.

**Recommend (a).** D65d keeps v2 only as the old path, and (b) is rewrite work on a path nobody is extending.

## D66d — approve the three contracts (answered)

**Answer (user): approve all three** (T154 `0771967`, T151 `f4a6155`, T153 `4bba422`).

- **T154** (server run loop, four `crpg-server` parts, S-set: it fixes the cycle order) — `task/T154-c1` `0771967`, revision 2. A paid Go review's four findings and its revision check's one finding are fixed.
- **T151** (Godot v4 session on the run loop) — `task/T151-c1` `f4a6155`. A free first-pass review (GLM-5.3-Flash) found nothing.
- **T153** (the AI host driver) — `task/T153-c1` `4bba422`. A free first-pass review (DeepSeek V4.1 Flash) found three real items: an ambiguous `capped` flag, a citation that did not support its claim, and an untested restore rule. The architect fixed all three, adding tests H8 (cap case) and H11.

**Recommend approving all three.**

## Agent log
- 2026-11-16 (UTC) · claude-code/opus-5.5 (manager) + session 67 decision batch · Drafted D66 from T154's two questions, T151 Question 5 and the three pending contract approvals, in plain language with a recommendation each, so the user can answer them in one pass.
- 2026-11-17 (UTC) · claude-code/opus-5.5 (manager) + user answers D66a–d · Recorded the user's answers (all four as recommended) so T154a, T151b and T153 can start from approved contracts.
