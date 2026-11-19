# Decision D69 — the saved script, dialogue and quest state (T052) questions and approval

Status: **open, 2026-11-19. Waiting for the user.**

T052 lets the game remember three things across a save, a quit and a load:
a scripted event that is waiting ("in 30 seconds, open the gate"), a
conversation a player is in the middle of, and how far each quest has got.
It only stores and counts. Running the scripts, conversations and quests is
later work (T053, T054). The contract is `tasks/T052.md` on `task/T052-c1`
(Opus architect, adjudicated from two blind drafts).

## D69a — should T052 change the save-file version number? (T052 Reading 8)

Every save file starts with a version number. Today it is 1, and the game
refuses any other number. T052 adds new kinds of information to the save.
Old saves still load correctly: missing information just means "nothing
waiting, no conversation, no quest started".

The catch is the other direction. If someone opens a new save in an *older*
build of the game, that build does not know the new information. It skips it
without any warning, so a waiting event, an open conversation or quest
progress would quietly vanish. Saves with creature and movement data already
have this problem today.

- (a) **Keep version 1 (recommended).** T052 stays a single change to the
  simulation code. A backlog item adds a proper "save schema version" later
  (the spec asks for one) that makes an older build refuse a newer save. It
  then covers creatures, movement and this state together.
- (b) **Raise it to 2 now.** Older builds would refuse new saves outright.
  But this needs a second small task in the server code. It also means
  changing an existing server test that checks "version 2 is refused", and
  that test change needs your approval. And the next time new information is
  added to saves, the same question comes back.

**Recommend (a).** Nobody has saves worth protecting yet, so a downgrade is
very unlikely. One general fix later is better than a single bump now.

## D69b — where does quest progress live for now? (T052 Reading 2c)

The engine runs each map area as its own "world". There is no
campaign-wide store yet. T052 keeps quest progress in the area's world,
because that is the only thing a save contains today.

- (a) **In the area's world for now (recommended).** This is enough for the
  MVP, which has one area and one quest. Before a second area is playable, a
  later task moves quest progress into a campaign-wide store. The data is
  just a list of "quest → current state", so the move is simple.
- (b) **Build the campaign-wide store first.** Quest progress would never be
  tied to an area. But that is a much bigger piece of work that nothing else
  needs yet, and it delays T052, the scripting that waits on it, and the MVP
  quest.

**Recommend (a).**

## D69c — approve the T052 contract

T052 is in the protected (S-set) category: it changes what the game world
stores and adds a step to every game tick. It has two parts:

- **T052** (`crpg-sim`): the three stores, the per-tick countdown of waiting
  scripts, the load checks, and the operations later script code calls.
  About 230 lines of code and 520 of tests, with a second, independent test
  writer.
- **T052b** (`crpg-server`, tests only): one new test. It saves a server to
  a file mid-wait, reloads it, and checks that it carries on exactly like a
  server that never stopped.

The countdown runs right after the combat clock, among the spec's "timers"
steps. It cannot change anything else the tick does. No existing save,
replay or test result changes.

**Recommend approve**, with the answers to D69a and D69b written into the
contract.

## Agent log
- 2026-11-19 (UTC) · claude-code/opus-5.5 (architect) + T052 contract · Drafted the T052 decision batch: the save-version question, the quest-scope question and contract approval, each explained plainly with a recommendation.
