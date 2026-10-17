# R12-deep — Event and trigger authoring for non-programmers (T099)

## Recommendations (ranked)

1. Ship the MVP on canned, parameterized quest/dialogue/trigger primitives plus wizards before any visual graph.
   - Why applies: A bare node graph is not enough; packaged high-level actions are what make no-code authoring usable.
   - Evidence: https://godotengine.org/article/godot-4-will-discontinue-visual-scripting/ — "Godot lacked high level components to make use of it."
   - Evidence: https://www.nwnlexicon.com/A_Simple_Quest — "without a single line of script-writing necessary."
   - Strength: vendor doc / practitioner report. Effort: medium. Status: confirmed from pass 1.

2. Keep the event IR declarative and frame-atomic, with bounded `Wait` continuations; no polling loops.
   - Why applies: State must stay consistent and save-safe; long-running or polled logic becomes a failure mode.
   - Evidence: https://docs.larian.game/Osiris_Overview — "Everything that happens in a single frame happens atomically..."
   - Evidence: https://fallout.wiki/wiki/Resource:Creation_Kit/Papyrus — "If you end up with >100 threads running at once..." [check: not found — V8]
   - Strength: vendor doc / practitioner report. Effort: medium. Status: confirmed from pass 1.

3. Make the IR canonical, deterministic text with stable IDs and a specified evaluation order; render the graph as a view.
   - Why applies: Graphs are hard to refactor and diff; a readable canonical form supports agents, Git, debugging, and round-trip import/export.
   - Evidence: https://codearchaeology.dev/languages/warcraft-3-gui/ — "see — irreversibly — the JASS their clicks had been generating all along"
   - Evidence: https://www.cirit.fi/blog/pros-and-cons-of-unreal-blueprints — "refactoring can be cumbersome as it often requires 'physically' moving nodes around..."
   - Strength: practitioner report. Effort: medium. Status: confirmed from pass 1.

4. Enforce time-slice budgets and no tick-based logic on event-driven paths.
   - Why applies: The common failure is hidden polling/native-call work, not node count.
   - Evidence: https://fallout.wiki/wiki/Resource:Creation_Kit/Papyrus — "the Papyrus scripting engine is designed to be used in an event-driven fashion." [check: not found — V8]
   - Evidence: https://dev.epicgames.com/community/learning/tutorials/wrKz/unreal-engine-fundamental-blueprint-practices — "Performing a lot of operations on blueprint Tick is one of the main reasons for the low performance..." [check: unavailable — V8]
   - Strength: practitioner report. Effort: small. Status: confirmed from pass 1.

5. Add state-driven event pages for simple appearance/availability, not just graph nodes.
   - Why applies: Non-programmers handle quest state well through conditional pages and switches/variables; this covers many trigger cases without graphs.
   - Evidence: https://rpgmakerofficial.com/product/MV_Help/page/01_09_01.html — "You can have one event with up to 20 event pages."
   - Evidence: https://rpgmakerofficial.com/product/MV_Help/page/01_09_01.html — "Map events occur during gameplay based on the contents of the page which meet these conditions."
   - Strength: vendor doc. Effort: small. Status: new.

6. Keep a layered authoring stack with one validation pipeline: wizard actions -> text IR -> sandboxed Lua.
   - Why applies: Novices need packaged actions, advanced authors need escape hatches, and agents must not bypass invariants.
   - Evidence: https://www.neverwintervault.org/project/nwn2/other/nwn2-toolset-how-writing-conversations — "The NWN2 toolset has a much more robust collection of global scripts that are ready for use in your own modules."
   - Evidence: https://solasta-dungeons.fandom.com/wiki/Solasta_Scripting_Language — "Solasta Scripting Language is an attempt to provide a more powerful toolset..."
   - Strength: practitioner report. Effort: medium. Status: confirmed from pass 1 / new layering detail.

7. Make diagnostics and debugging first-class and machine-readable.
   - Why applies: Novice-friendly tools fail on understanding rule flow; execution highlighting, references, and pointable errors recover them.
   - Evidence: https://www.sbgames.org/proceedings2021/ComputacaoFull/218250.pdf — "highlight rules as they are triggered in the game" [check: not found — V8]
   - Evidence: https://codearchaeology.dev/languages/warcraft-3-gui/ — "A map maker could select a trigger and choose Edit → Convert to Custom Text..."
   - Strength: practitioner report. Effort: medium. Status: new / confirmed from pass 1 diagnostics idea.

## What the research literature says

- Kelleher & Pausch, "Lowering the barriers to programming" (ACM Computing Surveys, 2005): visual tools reduce syntactic error barriers but still leave cognitive gaps; systems differ in whether they teach programming or empower users. https://www.cs.cmu.edu/~caitlin/papers/NoviceProgSurvey.pdf
- Green & Petre, "Usability analysis of visual programming environments" (J. Visual Languages & Computing, 1996): designers must trade off viscosity and secondary notation; VPLs can have good closeness of mapping but resistance to change. https://web.engr.oregonstate.edu/~burnett/CS589and584/CS589-papers/CogDimsPaper.pdf
- Green & Petre (via Wikipedia, Cognitive dimensions of notations): "there is no perfect interface and that trade-offs are a fundamental part of usability design." https://en.wikipedia.org/wiki/Cognitive_dimensions_of_notations
- Souza et al., "Games by End-Users: Analyzing Development Environments" (SBGames 2021): GDevelop's cause-consequence event model is easier for first contact, while Stencyl block programming gives more freedom at higher learning cost. https://www.sbgames.org/proceedings2021/ComputacaoFull/218250.pdf
- Druga, Ball & Ko, "How families design and program games" (IDC 2022): rule-based visual programming struggles with continuous motion state and rule conflicts; debugging support that highlights triggered rules is requested. https://doi.org/10.1145/3501712.3529724 [check: unavailable — V8]
- Mayer & Kuncak, "Pong Designer" (PLDI demo, 2012): demonstration-inferred rules allow pause/rewind and rule modification, but still expose rule conflict issues. https://infoscience.epfl.ch/server/api/core/bitstreams/c2e4fa08-5cb3-4d8b-b3fb-99319d2bcda9/content
- Cox, Murray & Salter, "Routine, Twisty, and Queer" (FDG 2025): accessibility is shaped by community identity and pedagogy, and AI-assisted visual scripting futures remain uncertain. https://doi.org/10.1145/3723498.3723817 [check: unavailable — V8]
- Blackwell & McIver surveys (PPIG 2002): cognitive evaluation of novice environments depends on error rates and environment support, not language labels alone. https://ppig.org/files/2002-PPIG-14th-mciver.pdf

## Pass-1 claims that did not hold

- Pass 1 said RPG Maker has a "20-page/6-condition" cap; the fetched official page confirms at most 20 event pages, but does not state a 6-condition limit. https://rpgmakerofficial.com/product/MV_Help/page/01_09_01.html [check: not found — V8]
- Pass 1 relied on an unreadable UESP Papyrus FAQ; it was replaced by the Fallout wiki Papyrus page plus the Papyrus efficiency forum thread. https://fallout.wiki/wiki/Resource:Creation_Kit/Papyrus
- Pass 1's framing that Blueprints empower non-coders at scale is too strong; the sources describe them as best for prototyping and component tasks, with refactoring, encapsulation, and complexity costs. https://www.cirit.fi/blog/pros-and-cons-of-unreal-blueprints

## Contradictions

- Graphs vs text: Blueprint practitioners value visual flow and discoverability (https://dev.epicgames.com/community/learning/tutorials/wrKz/unreal-engine-fundamental-blueprint-practices), while others find text more compact and maintainable under refactoring (https://www.cirit.fi/blog/pros-and-cons-of-unreal-blueprints). [check: unavailable — V8]
- Wizards vs code: NWN2 demonstrates no-code go-fetch quests (https://www.nwnlexicon.com/A_Simple_Quest), while its own docs say conversation scripting is complex and benefits from specialists (https://www.neverwintervault.org/project/nwn2/other/nwn2-toolset-how-writing-conversations).
- Bare graphs vs packaged components: Godot attributes VisualScript's failure to missing high-level components (https://godotengine.org/article/godot-4-will-discontinue-visual-scripting/), while HCI research says visual programming can work where integrated with a domain. https://web.engr.oregonstate.edu/~burnett/CS589and584/CS589-papers/CogDimsPaper.pdf

## Not applicable

- Save-time thread persistence and patch-time function locking belong to Bethesda's VM, not to our budgeted deterministic IR.
- StarCraft's 2-second polling cycle is strategy-map scheduling, inappropriate for turn-based CRPG interaction logic.
- Solasta GM-mode database deletion is play behavior, not authoring semantics for campaigns.
- Warcraft vJASS/coffee preprocessors are community tooling history, not a requirement for our compiler-owned transforms.
- Osiris's alphabetical goal ordering shows why ordering must be explicit metadata, not a side effect of source layout.

## Agent log

- 2026-10-16 (UTC) · claude-code/bookkeeper + T099 fold-in · Keeps the deep pass's event and trigger authoring findings as the T099 record; markers flag the claims the V8 check could not verify.
