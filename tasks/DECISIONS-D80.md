# Decision D80 — walk, then fight, revision 2 (T157)

Status: **open, waiting for the user.** Contract `tasks/T157.md` revision 2
(architect) is written for both recommended options (a); each question says
what changes otherwise. The user also accepts or returns two drafted design
records: ADR-0029 Amendment 1 (sides, the end of a fight, engagement) and
ADR-0036 Amendment 1 (survivors walk again), both appended with status
Proposed.

## What you would see
Start `Adventure.tscn`. Two heroes stand in the room; a goblin waits at the
far side. Walk with left clicks, stop with right clicks. Walk a hero to
within 4 m of the goblin and the fight starts by itself in place (or click
the goblin to start it sooner). The goblin, played by the server's AI, walks
up to 6 m toward the nearer hero and strikes if it reaches. On a hero's turn
a line at the bottom says "Movement: 6 / 6"; click the floor to step (the
line drops as you spend it; a click too far is refused and the log says so),
click the goblin to strike once within 1.5 m, or press "End turn". When the
goblin falls, the fight is over because only the heroes' side is left, and
both heroes walk again.

How sides work (decided in the contract, from your D79 answer): a
creature's side is its faction, which campaign data can already name. The
two heroes are in the faction "Heroes", the goblin in "Goblins". A creature
with no faction fights alone, which is exactly how every fight behaves
today, so no existing fight or recording changes.

## Questions

**Q1 (D80a) — Where does the "notice distance" live?** It is how close a
hero must come to an enemy before the fight starts by itself.
- **(a) On the encounter (recommended).** Each authored fight says "this
  starts when anyone comes within 4 m of an enemy". One number per fight, in
  the file that already lists who takes part; easy to author and to test.
- **(b) On each creature.** Each monster type has its own sight range (a
  bat notices you at 10 m, a sleeping troll at 1 m), and the fight starts
  when any creature sees an enemy. More realistic, but every creature needs
  a value and the rule must pick between two creatures' ranges; the data
  task moves to the creature file.
- **(c) One value for the whole ruleset.** Simplest data, but every fight in
  a campaign starts at the same distance.

**Q2 (D80b) — May two existing tests change so that survivors walk again?**
When a turn-based fight ends, the engine's movement rule still says the
survivors are "in a turn-based fight" and refuses to let them walk, and two
existing tests pin exactly that. Your D79 answer covers the rule for when a
fight ends, not this one, so it needs its own yes.
- **(a) Yes: a fight with nobody's turn left holds no one (recommended).**
  One condition changes in the simulation and the same one in the server's
  movement check, matching how a finished real-time fight already behaves.
  The two named test lines change their expected answer (from "refused" to
  "allowed"), and nothing else changes. This is ADR-0036 Amendment 1.
- **(b) No; a finished fight switches back to real time instead.** No pinned
  test changes, but the switch sits in the end-of-fight rule, may change
  recorded baselines (those then need your review), and fights in rulesets
  without real time would still freeze their survivors. The contract goes
  back for a revision of three parts.
- **(c) No; wait until the server can close finished fights properly.** The
  demo ends at "fight over" and the heroes stay put until a later server
  task (closing fights also needs the health bars and corpses to survive it).

## Not questions (decided in the contract)
- The server AI keeps getting its "who is on which side" list from the
  bridge; with factions it agrees with the engine's sides. Having the server
  derive it from the engine is the existing backlog row (your D78a long-term
  choice), now unblocked, and changes two callers, so it is a follow-up.
- Faction relations (allied or neutral factions) are not read yet: any two
  different factions are enemies. A follow-up.
- The movement line shows pool points (one point is one metre in the demo);
  metres, a reach ring and a path preview are a follow-up.
- Clicking your own hero during a fight never attacks it; the HUD buttons
  still offer every legal option.
- The four new bridge functions (`open_v4_walk`, `request_mode`, and the
  view's `movement` and `movement_max`) are on your review list, as before.

## Answers (the user, 2026-11-22)
- **D80a: (a) on the encounter, as the default for now**, with the user's condition: "It will be variable. On the encounter is fine as a default now but there needs to be different ways for different rulesets along with alternate ways to trigger fights." The data shape and ADR text must leave room for other trigger kinds and per-ruleset choices without a schema break.
- **D80b: (a) yes**, the two named pinned assertions change.
- **ADR-0029 Amendment 1 and ADR-0036 Amendment 1: accepted.**

Status: **answered.** Revision 3 makes the engagement trigger extensible.

## Agent log
- 2026-11-22 (UTC) · claude-code/opus-5.5 (architect) + T157 revision 2 · Wrote the two user questions revision 2 leaves open (where the notice distance is authored, and whether two pinned tests may change so survivors walk again), each with its recommendation, and listed the sides and readout defaults decided in the contract.
- 2026-11-22 (UTC) · claude-code/opus-5.5 (manager) + D80 answers · Recorded the user's answers and their condition that fight triggers be variable per ruleset, so revision 3 builds on them.
