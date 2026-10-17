# R8-deep.md — Module-editor prior art (T099 deep pass)

## Recommendations

1. **Ship an interactive guided walkthrough for the first module, not just docs.** Why: §17.2 DoD requires a first-time author to complete an area, NPC, conversation, quest and fight in one pass; game-making complexity makes teaching necessary. Evidence: https://arxiv.org/abs/2006.03519 — "Interactive Help has a greater positive impact on time spent, controls learnability, learning motivation, total editor activity, and game level quality." (`measured study`; `medium`). Mark: `confirmed from pass 1`.
2. **Keep the default quest/NPC loop declarative via linkable gadgets and conditions; keep Lua for advanced scripts only.** Why: serves both non-programmer and agent authors and matches §4.8/§11.4 graph-first triggers. Evidence: https://steamcommunity.com/app/1096530/discussions/4/3278069673189411054 — "Scripts are avoided to make it user friendly ... there are map gadgets that can be linked together." (`practitioner report`; `medium`). Mark: `confirmed from pass 1`.
3. **Make the continuous Problems panel and headless `validate` a core quality feature before visual polish.** Why: dangling references, wrong event attachment and unreachable quest states are the failure mode that blocks non-programmers; CONTEXT.md §11.4 already plans this. Evidence: https://nwn.fandom.com/wiki/Toolset — "NWScript is similar to C and Java, which can make NWScript daunting for the novice scripter." (`practitioner report`; `medium`). Mark: `confirmed from pass 1`.
4. **Add crash-safe saves: auto-save, recovery and external backups; test a released copy separately.** Why: editors crash and JSON campaigns must survive; matches §11.2 durability gap. Evidence: https://nwn.wiki/spaces/NWN1/pages/60982732/Toolset+User+Manual — "The toolset can crash from time to time, so make sure you make regular saves. It is worth keeping a copy of your module in a separate location." (`practitioner report`; `small`). Mark: `confirmed from pass 1`.
5. **Scaffold every object as a structurally valid document and never start from blank.** Why: agents and novices need a working baseline; supports `crpgc new` and generated property forms (§4.8 #3, §11.4). Evidence: https://nwn.fandom.com/wiki/Your_first_module — "Once you have completed the module wizard you should have a shiny new (and empty) module with a single area in it." (`practitioner report`; `small`). Mark: `confirmed from pass 1`.
6. **Keep campaign data as canonical, diffable JSON rather than a binary module file.** Why: external Git/workflow is already a stated community need and our document model allows agents to inspect and diff changes. Evidence: https://www.reddit.com/r/neverwinternights/comments/1d8sjj6/how_do_you_collaborate_to_build_modules_in_aurora/ — "You convert the files to json format using neverwinter.nim ... the git recognizes the diff in the json format and you can commit your changes." (`practitioner report`; `medium`). Mark: `new`. [check: unavailable — V8]
7. **Support inheritance/template overrides; avoid copy-only object creation.** Why: `base: "goblin"` with overrides is a stated principle (§11.4 creature editor); locking authors to clones of existing assets is a recurrent complaint. Evidence: https://steamcommunity.com/app/1096530/discussions/4/3278069673189411054 — "Items and NPC (and mobs) can be created, but they must be based off existing items or NPCs." (`practitioner report`; `small`). Mark: `confirmed from pass 1`.
8. **Prompt one good help facility and keep the first loop under one session; do not surface too many help choices.** Why: complexity-dependent benefit and choice overload are well measured; supports MVP scope (§17.2/#17.3). Evidence: https://grail.cs.washington.edu/projects/game-abtesting/chi2012/chi2012.pdf — "tutorials increased play time by as much as 29% in the most complex game" and "investment in tutorials may not be justified for games that can be learned through experimentation." (`measured study`; `small`). Mark: `new`.
9. **If PCG or AI co-creation is added later, expose it through example/region selection, not manual rule editing.** Why: later-phase experimentation should stay usable by non-technical authors; manual grammar or AI roles otherwise fragment the workflow. Evidence: https://dl.acm.org/doi/fullHtml/10.1145/3555858.3555895 — "allowing designers to specify a region in the expressive range of outcome driven metrics is both a viable means of providing control over these systems and is easier to use than interacting manually with grammars rules." (`measured study`; `large`). Mark: `new`. [check: unavailable — V8]

## What the research literature says

- Kao (2020, n=1,646) found Interactive Help improved time spent, learnability, motivation, editor activity and level quality; text documentation is most common (89.4%) but weaker alone: https://arxiv.org/abs/2006.03519.
- Andersen et al. (2012, 45,000+ players) found tutorials increased play time up to 29% in the most complex game but not significantly in simpler games; context-sensitive timing increased play time 16% and progress 40%: https://grail.cs.washington.edu/projects/game-abtesting/chi2012/chi2012.pdf.
- Guzdial et al. (2019) found level designers vary in desired AI role (friend, collaborator, student, manager), adapt to the tool, and value it as an inspiration source: https://arxiv.org/abs/1901.06417.
- StarCharM (2024) shows a wizard-style GenAI modding workflow can lower technical barriers but raises concerns about originality and community bonds: https://dl.acm.org/doi/pdf/10.1145/3748612. [check: unavailable — V8]
- Mechanic Maker (2024) shows example-based synthesis lets non-programmers create mechanics as effectively as programmers: https://arxiv.org/html/2410.01096.
- Example-selection control (2023) shows non-technical designers can steer procedural generators by selecting example regions instead of editing grammar rules: https://dl.acm.org/doi/fullHtml/10.1145/3555858.3555895. [check: unavailable — V8]
- RL Brush (2020) users who accepted AI suggestions produced more playable and more complex levels: https://arxiv.org/abs/2008.02778.
- Sentient Sketchbook (2013) computer-assisted authoring can automate playability checks and generate alternatives to a designer's map sketch: https://www.um.edu.mt/library/oar/handle/123456789/29607.
- Gregory (2018) treats the world editor and tools as part of the asset pipeline/gameplay foundation, not an afterthought: https://www.oreilly.com/library/view/game-engine-architecture/9781351974271/contents.xhtml. [check: unavailable — V8]
- The Level Design Book treats editors as engine-specific tooling; built-in 2D editors are preferred when available and scripting handles objectives/choreography: https://book.leveldesignbook.com/appendix/tools.

## Pass-1 claims that did not hold

- Pass 1 cited Foundry users as overwhelmed for weeks; the same discussion also contains a user who was set up in "30 minutes" and running "maybe another 4hrs total" (https://www.reddit.com/r/FoundryVTT/comments/1g8akpv/fairly_disappointed_with_foundryvtt_so_far). Learning time is experience-dependent, not inherently month-long. [check: unavailable — V8]
- Pass 1 treated a fixed 19-step tutorial model as onboarding guidance; Kao's measured result says offering multiple help types together worsened outcomes compared with one prompted facility (https://arxiv.org/abs/2006.03519).

## Contradictions

- **Setup time:** one Foundry user says "at least a couple months" (https://www.reddit.com/r/dndnext/comments/rttacn/is_foundry_vtt_worth_it) while another says "It took me maybe 30 minutes to get set up and then maybe another 4hrs total" (https://www.reddit.com/r/FoundryVTT/comments/1g8akpv/fairly_disappointed_with_foundryvtt_so_far). [check: unavailable — V8]
- **Tutorial value:** Andersen et al. (2012) found tutorials significantly helped the most complex game but did not significantly improve engagement in simpler games (https://grail.cs.washington.edu/projects/game-abtesting/chi2012/chi2012.pdf).
- **Scripting:** NWN wiki describes NWScript as "daunting for the novice scripter" (https://nwn.fandom.com/wiki/Toolset), while Solasta practitioners describe gadgets replacing scripts and say the maker is "close... getting better" but "not as good as NWN" (https://steamcommunity.com/app/1096530/discussions/4/3278069673189411054).

## Not applicable

- External master-file, hak-pack and game-data-path setup: our flow is a single campaign document plus an in-process authoritative server.
- Copy-only entity creation: our schema-driven documents can define new entities; templates are convenience, not a hard limit.
- Procedural AI map generation as primary editing: the MVP needs authored, navmesh-backed levels with deterministic tests.
- Month-long learning curves: MVP DoD requires the first scenario to be completable in one session.
- Mod-marketplace or plugin-ecosystem framing: our differentiator is campaign authoring and the headless agent API.
- Foundry-style separate import, hosting and permission workflows: our editor owns save, load, test and validation in one process.
- Scripting-first authoring for core MVP quests and dialogue: keep the base loop declarative and reserve Lua for advanced behavior.
- Up-front manuals over built-in help: Kao's evidence favors integrated, prompted interactive help for complex editors.

## Agent log

- 2026-10-16 (UTC) · claude-code/bookkeeper + T099 fold-in · Keeps the deep pass's module-editor prior art as the T099 record; markers flag the claims the V8 check could not verify.
