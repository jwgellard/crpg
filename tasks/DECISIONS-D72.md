# Decision D72 — the agent authoring benchmark (T100) questions

Status: **open, 2026-11-19. Waiting for the user.**

T100 builds the test bench for your "agents and humans must find campaign
authoring easy" concern (D28j). It gives several AI models a plain task such
as "build a small campaign with two fighters and one fight", with nothing but
the engine's command-line tool (`crpgc`) and the schema files, and then
scores what they wrote: does it pass the validator, did they write all the
parts the task asked for, does its fight actually start, how many steps did
it take, and how many `crpgc` commands failed along the way. The first
measurement is on today's tools (the baseline); the authoring design the
architect writes next (T101) is then re-measured the same way, so the design
is judged by numbers, not by taste. It includes the "define a small rule
system" task you asked for (D28l). The contract is `tasks/T100.md` on
`task/T100-c1` (Claude architect, adjudicated from two blind drafts); it is
two Python tool parts (no engine code), so it is not in the protected
category. No new library is added in any answer. Neither question blocks
building the tool: a different answer is a small fix afterwards.

## D72a — what counts as a campaign being "playable"? (T100 Reading 4)

The plan row says playability is checked with `crpgc run`. But `crpgc run`
never opens a campaign: it runs an empty test world and prints fingerprints of
it. Every campaign, good or broken, would "pass" it. What does open a campaign
and start its first fight with the real rules is `crpgc play`, which normally
waits for a person to type a number for each move. The benchmark can feed it
the number "1" up to 400 times, so no person is needed.

- (a) **Keep it literal: `crpgc run`.** The score would not depend on what the
  agent wrote, so it measures nothing about the campaign.
- (b) **Use `crpgc play` with scripted answers (recommended).** The fight must
  start, with the number of fighters the task asked for. Whether it also
  finishes is recorded but not required (always choosing "1" need not win).
  `crpgc run` still runs once per scoring as a health check that the tool build
  works. Limit: only fights can be played today, so dialogue, quests and items
  in the biggest task are checked by the validator and counted, but not played.
- (c) **Both must pass.** In practice the same as (b), because `crpgc run` does
  not depend on the campaign.

**Recommend (b).** Under (a) or (c) the "playable" score changes to the
weaker definition (one function in the scorer and its tests), and the benchmark
loses its only measure of whether the agent's campaign starts.

## D72b — what does the agent get to start with? (T100 Readings 3 and 5)

The baseline measures "today's tools", so what the agent is handed decides
what the numbers mean. `crpgc` prints no help text of its own, and its
`schema` command prints only 17 of the 23 schema files: ability, effect,
encounter, outcome table, ruleset and UI have no command, yet a playable
campaign needs four of them.

- (a) **`crpgc`, a copy of the 23 schema files, and a plain list of the six
  authoring commands with their usage lines; no description of the folder
  layout (recommended).** The agent must find out where each kind of file
  goes mostly by trying and reading the validator's messages. That shows how
  discoverable the layout is today, which is probably the biggest agent
  problem. The four from-scratch tasks may score low as a result; the fifth
  task (add a creature to a copy of a valid campaign, which shows the layout)
  still shows what works when the layout is visible. The list of commands sits
  in one file, so testing a new command design later means giving the bench a
  different list and a different `crpgc` build, with no code change.
- (b) **As (a), plus a short cheat-sheet of the folder layout.** Higher
  success, but it hides the discoverability gap that the authoring design most
  needs to measure.
- (c) **`crpgc` only, no schema files.** The closest reading of "only `crpgc`
  and the schemas" if "the schemas" meant the `crpgc schema` command. But the
  agent could then never learn the shape of an ability, an outcome table, a
  ruleset or an encounter, so every task but the first would fail for a reason
  no change to the authoring commands could fix.

**Recommend (a).** If the baseline comes out near zero everywhere, (b) is one
more benchmark run with another list file and costs no code.

## Agent log
- 2026-11-19 (UTC) · claude-code/sonnet-5.5 (architect) + T100 contract · Drafted the T100 decision batch: what "playable" means when `crpgc run` never opens a campaign, and what the benchmarked agent is handed, each explained plainly with a recommendation.
