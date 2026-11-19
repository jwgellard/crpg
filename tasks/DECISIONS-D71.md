# Decision D71 — the headless-client and desync suite (T041) questions and approval

Status: **open, 2026-11-19. Waiting for the user.**

T041 builds test clients that play the game without Godot, the way the real
client will, and uses them to prove four things the M4 milestone asks for:
the game stays in agreement over a bad network for 5,000 ticks (20 Hz,
about four minutes of play), two players see the same world, a cheating
client's messages are refused cleanly, and the real dedicated server
program works with two clients on both Windows and Linux (gate 13). It is
all test code: no game code changes, no new library. The contract is
`tasks/T041.md` on `task/T041-c1` (Opus architect, adjudicated from two
blind drafts).

## D71a — should the dedicated-server smoke test also move characters? (T041 Question 1)

M4's first exit criterion says two scripted clients connect over the real
network to a dedicated server, **move** and fight, and end up seeing the
same thing. The dedicated server program you approved in T040 serves the
combat connection only. It has no movement connection yet, because a
movement connection cannot yet reconnect after a dropped line (T040
Reading 8). So the clients in T041's dedicated test can connect, fight and
agree, but cannot move. Movement is still proved, in-process, over the same
server code (the desync run, the real-time walk and the embedded fight).

- (a) **Fight only for now; add movement when the server can serve it
  (recommended).** When the movement-connection backlog item T040 filed is
  done, a small follow-up (T041e, one more test in the same file) adds
  moving. Until then M4's first criterion is partly open. T040 does not
  change.
- (b) **Also test real-network movement now, without the server program.**
  A test starts the server code in-process on a movement connection with
  two real network clients. Movement over the real network is proved
  sooner, but not on the program players will run, and it brings back the
  test shape (many real connections in one test process) behind past
  intermittent CI failures (T098).
- (c) **Reopen T040** so the program also serves movement. Its approved
  contract changes, and movement reconnection has to be built first.

**Recommend (a).** It keeps T040 as you approved it and keeps the flaky
test shape out; the only thing it defers is the one step that cannot be
built yet.

## D71b — approve the T041 contract

Four parts, each one crate and one worker run, with a Claude reviewer
planting bugs to check the tests catch them:

- **T041a** (`crpg-testkit`, about 600 support + 300 test lines): the test
  client, a simulated network with 3 % loss, delay, jitter, resends and
  duplicate packets, and a test "arena" that runs the real server loop.
- **T041b** (`crpg-testkit`, about 450 test lines): the 5,000-tick run (two
  players patrolling, an observer trying to cheat every 250 ticks), each
  client compared with an identical fault-free "twin" connection and the
  server checked against an independent copy of the world; a repeat run
  must give the same result; a broken network must be caught; a real-time
  walk; and two players who move and fight to the death and agree
  (gate 13's single-player leg).
- **T041c** (`crpg-testkit`, about 500 test lines): ten cheating-client
  cases (acting for someone else's character, acting out of turn or with a
  stale clock, targeting what you cannot see, flooding, oversized and
  garbled messages, replays), including the case T042 handed over: a
  replay spelled differently but meaning the same must count as a replay,
  not as cheating.
- **T041d** (`crpg-server` tests, about 380 lines; **waits for T040**):
  starts the real server program, two real network clients fight to the
  same final state, one cheating message must show up as the server's log
  line, and `quit` must save (gate 13's Windows and Linux dedicated legs).

T041a–c can start now. T041d starts after T040's program is built. No new
dependency and no CI change: the tests run in the existing test job on
both systems.

**Recommend approve**, with D71a's answer written into the contract.

## Agent log
- 2026-11-19 (UTC) · claude-code/opus-5.5 (architect) + T041 contract · Drafted the T041 decision batch: whether the dedicated-server smoke must also move characters before the server can serve movement, and contract approval, each explained plainly with a recommendation.
