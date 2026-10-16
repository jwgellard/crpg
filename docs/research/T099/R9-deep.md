# R9-deep — Narrative tools: dialogue and quest authoring

## Recommendations

### 1. Keep the node graph primary; keep the indented text syntax a lossless round trip (`confirmed from pass 1`)
- Recommendation: Keep the node graph as the primary authoring surface; the indented text form should import/export losslessly, preserving node IDs, choice order, conditions, comments and test metadata.
- Why it applies: `CONTEXT.md` commits to both views, and a 1:1 conversion (as articy does for screenplay text) avoids dropped nodes or condition badges during AI/CLI edits.
- Evidence (`vendor doc`): https://www.articy.com/en/articydraft/feature-list — "**Convert document to Flow** – Convert screenplay format text into a visual representation in the Flow editor just by drag and drop."
- Evidence (`vendor doc`): https://github.com/inkle/ink/blob/master/Documentation/WritingWithInk.md — "**ink** is a scripting language built around the idea of marking up pure-text with flow in order to produce interactive scripts."
- Effort: medium

### 2. Make “seen / visited / read count” a first-class condition and quest operand (`confirmed from pass 1`)
- Recommendation: Add built-in operands for “this node/state has been reached” and “read count of this node/quest state,” alongside variables.
- Why it applies: It supports condition badges and reachability panels without forcing writers to maintain tracking booleans, and matches Ink’s address-as-test model.
- Evidence (`vendor doc`): https://github.com/inkle/ink/blob/master/Documentation/WritingWithInk.md — "Every knot/stitch in the game has a unique address (so it can be diverted to), and we use the same address to test if that piece of content has been seen."
- Evidence (`vendor doc`): https://twinery.org/cookbook/passagevisits/harlowe/harlowe_passagevisits.html — "Combined together, the macro `(count:)` with the `(history:)` can return the number of times a certain passage has been visited during the course of the story."
- Effort: medium

### 3. Make “walk this conversation” a real debugger, not a passive playthrough (`confirmed from pass 1`)
- Recommendation: Implement jump-to-node, live variable watch, force-assigning the Previous value, undo of the last choice, quickstart from any node, and condition-aware branch evaluation.
- Why it applies: `CONTEXT.md` already plans the simulator; Arcweave shows that force-assign and edit-while-playing materially improve testing logic.
- Evidence (`vendor doc`): https://docs.arcweave.com/project-tools/play-mode/overview — "Ability to inspect variable changes and force assign values for testing."
- Evidence (`vendor doc`): https://docs.arcweave.com/project-tools/play-mode/debugger — "Editing a variable's **Previous value** allows you to test the logic of the current element."
- Evidence (`vendor doc`): https://docs.arcweave.com/project-tools/play-mode/overview — "Quickstart Play Mode from any element: Directly playtest any part of the story."
- Effort: medium

### 4. Keep continuous structural validation, with a concrete check list (`confirmed from pass 1`)
- Recommendation: Validate on every change for orphan nodes, undefined variables, loose ends, unreachable dialogue/quest states, and audio/tag mismatches.
- Why it applies: This is the promised Problems panel; static checks plus compiler warnings catch errors before save or play.
- Evidence (`vendor doc`): https://www.pixelcrushers.com/dialogue_system/manual2x/html/dialogue_editor.html — "Check For Issues: Run reports to check for issues such as orphaned dialogue entry nodes that aren't linked from anywhere."
- Evidence (`vendor doc`): https://github.com/inkle/ink/blob/master/Documentation/WritingWithInk.md — "WARNING: Apparent loose end exists where the flow runs out. Do you need a '-> END' statement, choice or divert? on line 3 of tests/test.ink"
- Effort: small

### 5. Store per-language status and auto-mark outdated text (`confirmed from pass 1`)
- Recommendation: Track final/in-progress/review/outdated state per localizable text/VO asset and auto-mark other languages outdated when source changes.
- Why it applies: `CONTEXT.md` currently only names missing localisation keys; status tracking and spread-sheet round trip make translation manageable.
- Evidence (`vendor doc`): https://www.articy.com/en/articydraft/feature-list — "When you change the reference property text, all other language texts will be marked as outdated automatically."
- Evidence (`vendor doc`): https://docs.arcweave.com/project-tools/localization — "The available statuses are: ... **Final** (green)"
- Effort: medium

### 6. Tie voice-over to stable line/document IDs from the start (`new`)
- Recommendation: Add localisation/VO asset linkage to stable IDs at authoring time rather than adding tags late.
- Why it applies: Persistent IDs reduce drift across translators and VO, and match our document/object ID model.
- Evidence (`vendor doc`): https://docs.yarnspinner.dev/2.4/using-yarnspinner-with-unity/assets-and-localization.md — "if you're building a game that's voice acted, it makes your life significantly easier if you build your systems with localisation in mind from the start."
- Evidence (`vendor doc`): https://www.pixelcrushers.com/dialogue_system/manual2x/html/localization.html — "If a localized field name is not defined for the current language, it will use the default field."
- Effort: medium

### 7. Add reachability and possibility-space summaries (`new`)
- Recommendation: Show not only greyed unreachable nodes but path summaries/counts of reachable states and impossible branch combinations.
- Why it applies: `CONTEXT.md` already plans reachability analysis; visualizing possible playthroughs helps authors keep graphs aligned with intent.
- Evidence (`measured study`): https://arxiv.org/abs/2601.15295v2 — "A user study (n=12) shows that our approach improves author anticipation of player-experienced narrative, leading to more effective control and exploration of the narrative possibility spaces."
- Evidence (`paper`): https://arxiv.org/abs/2305.01925v5 — "It presents an overview of Interactive Digital Narratives (IDNs) design for complexity representations through IDN workflows and methodologies, IDN authoring tools and applications."
- Effort: medium

### 8. Expose agent edits through receipts and structural review (`new; extends pass 1`)
- Recommendation: Keep GUI and agents on the same transactional command surface, with receipts, rollback, and structural review for every change.
- Why it applies: `CONTEXT.md` already plans `apply`/receipts; Narrative Steward and our command API converge on this.
- Evidence (`paper`): https://arxiv.org/abs/2609.39333v1 — "change records and execution verification help authors assess the resulting work."
- Evidence (`vendor doc`): https://nwn.wiki/spaces/NWN1/pages/60982751/Conversation+Editor — "Of course this test does no starting conditional Text Appears When... checks, instead of all options both PC and NPC are shown in the window (which can be a bit confusing)."
- Effort: small

## What the research literature says

- Perkis et al. (2023), INDCOR White Paper 2, arXiv:2305.01925: maps IDN authoring tools, workflows and complexity. Quote: "IDN workflows and methodologies, IDN authoring tools and applications."
- Nack et al. (2023), INDCOR White Paper 3, arXiv:2306.10547: interaction complexity spans authoring, delivery, and audience perception. Quote: "complexities of authoring, delivering and processing dynamic interactive contents from the perspectives of both creators and audiences."
- Clerc et al. (2025), "Modeling Interactive Narrative Systems," arXiv:2508.05653: formal vocabulary helps compare systems. Quote: "provides a consistent vocabulary and modeling structure."
- Wang et al. (2026), "Elsewise," arXiv:2601.15295: possibility-space visualization improved anticipation/control (n=12). Quote: "improves author anticipation of player-experienced narrative."
- Wang et al. (2026), "NarrativeSteward," arXiv:2609.39333: linked artifacts, review, change records improve agent-assisted authoring (n=12). Quote: "easier formulation of revision requests and inspection of changes."
- Sun et al. (2025), "Drama Llama," arXiv:2501.09099: natural-language triggers can replace handwritten logical preconditions (n=6). Quote: "authors define triggers in natural language."
- Chen et al. (2024), "ConnectVR," arXiv:2406.15889: trigger-action authoring made causal narrative chains manageable (workshop n=15 plus two artists). Quote: "trigger-action interface to enable non-technical creators design."
- Lu et al. (2025), "WhatELSE," arXiv:2502.18641: narrative space can be shaped at configurable abstraction levels. Quote: "shaping narrative spaces at configurable level of abstraction."

## Pass-1 claims that did not hold

- "Yarn Spinner Story Solver checks logic and soft locks" (pass 1 table) is not supported by any fetched page; our loaded Yarn docs describe syntax and variable storage, not soft-lock verification. Treat it as unverified.
- The pass-1 implication that a built-in tool test can stand in for game testing is contradicted: NWN’s editor test skips initial conditionals and displays all options, so it cannot replace an in-game/run test.

## Contradictions

- Graph-first vs text-first: articy converts "screenplay format text into a visual representation," while Ink argues its "script aims to be clean and logically ordered, so branching dialogue can be tested 'by eye'".
- Authoring-layer state vs game-layer storage: Dialogue System/Pixelcrushers/articy expose variables, conditions, inspection in the authoring layer; Yarn delegates variables to a "variable storage object" supplied by the game.
- Validation convention: NWN’s built-in test "does no starting conditional Text Appears When... checks," whereas Dialogue System/Ink provide compile/static checks (undefined variables, loose ends).
- Text round trips can lose editor truth: positions, comments, status flags, VO metadata and condition badges are not necessarily representable as plain text unless the schema defines them.

## Not applicable
- Our canonical game state remains JSON campaign documents; no source syntax replaces them.
- No loaded source gives measured editor productivity data for our exact Rust/GDScript stack.
- No evidence favors replacing conditions/quests with natural-language triggers or LLM-generated narratives.
- Our MVP limits localisation/VO to documents; runtime localisation portals are out of scope here.

## Agent log

- 2026-10-16 (UTC) · claude-code/bookkeeper + T099 fold-in · Keeps the deep pass's narrative-tool findings as the T099 record; the V10 check confirmed every claim, so no markers were needed.
