# Decision D78 — the Godot combat demo (T156)

Status: **decided by the user, 2026-11-22: D78a (a), D78b (a), D78c (a), D78d (a); contract `tasks/T156.md` approved.** D78a was taken after the user asked what is best long term: `Host::drive_ai` already takes `crpg_ai::Sides`, so a re-export only renames it; the long-term fix (the server derives AI sides from campaign data) is a backlog follow-up. Contract `tasks/T156.md` (architect,
T156c attempt 1), written for every recommended option (a); each question
says what changes otherwise.

## What you would see
Start the client and the fight opens as today, but the Goblin's turns are
now played by the same AI the dedicated server uses, running inside the
game's own background server. When it is your turn, click the Goblin to
strike it (the buttons still work). A health bar floats over each head, a
red "-4" or a grey "miss" rises from whoever was hit, and the target flashes
white for a quarter of a second.

One correction to the brief: the Goblin already takes its turns today. The
game's bridge plays it with a small stand-in rule ("first attack on another
side, after 0.6 s"). It runs inside the background server, not in the game
script, but it is not the server's AI. T156 swaps in the real AI.

## Questions

**Q1 (D78a) — May the Godot bridge use the AI library directly?** To hand the
server's AI its "who is on which side" list, the bridge needs that list's
type, which lives in the AI library (`crpg-ai`). Using it is one line in the
bridge's manifest and one line in the lock file. It adds no outside code: the
AI library is ours and is already compiled into the game through the server
library, and the layering check already allows the link.
- **(a) Yes, link the bridge to the AI library (recommended).** One package
  fewer, nothing in the server changes.
- **(b) No; the server library re-exports the type instead.** A tiny extra
  server task runs first, and the server's public surface grows by three
  names it does not otherwise need to expose.

**Q2 (D78b) — How fast should the enemy play?** The AI decides one step at a
time (attack, then end the turn). The server can wait between steps so you
can follow them; this waiting changes nothing about what happens, only when.
- **(a) 0.6 s per step (recommended).** The same pace as today's Goblin: its
  attack lands 0.6 s into its turn and your turn starts 0.6 s later.
- **(b) 0.3 s per step.** Snappier; numbers and flashes overlap more.
- **(c) As fast as the server runs (about 0.1 s per step).** The enemy's turn
  is over almost before you see it, as on the dedicated server today.

**Q3 (D78c) — What does clicking an enemy do?**
- **(a) Attack at once (recommended).** One click sends the first attack the
  server offers against that character; if it offers none (not your turn,
  out of reach), a short note appears in the log and nothing is sent.
- **(b) Select, then confirm.** The first click marks the target; a second
  click or a button confirms. Safer against misclicks, but it needs a
  selection marker and another round on this contract.

**Q4 (D78d) — Which scene becomes the combat demo?**
- **(a) The default scene, `Main.tscn` (recommended).** It is what opens when
  you start the client, and the automatic Windows build check already plays
  it to the end, so that check will prove the AI drives the enemy on every
  change. The existing tests stay valid unchanged.
- **(b) A new `Combat.tscn`, launched by a command.** The default scene stays
  exactly as it is; the new scene is checked only by a local test until a
  later task adds it to the build check.

## Not questions (decided in the contract)
- The AI controls every combatant not on your side (who the enemy is today).
- The bridge change to the game boundary (one new `open_v4_ai` function) is
  on your review list: T156b waits for your review, as T050b did.
- Running the two new client tests in the build check is a later workflow
  task (also on your review list).

## Agent log
- 2026-11-22 (UTC) · claude-code/opus-5.5 (architect) + T156c · Wrote the four user questions for the combat demo (the AI library link, enemy pace, click behaviour, which scene), each with its recommendation, and corrected the brief's premise that nobody drives the enemy today.
- 2026-11-22 (UTC) · claude-code/opus-5.5 (manager) + D78 answered · Recorded the user's answers (all (a)), the contract approval, and the long-term reasoning behind D78a.
