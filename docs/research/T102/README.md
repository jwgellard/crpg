# T102 — clickable prototype of the M6 editor flows

## How to open
Open `docs/research/T102/prototype/index.html` in any browser (double-click / "Open with"). It is a single self-contained file: inline CSS and JavaScript, no external script, font, image or network request. Works opened from disk (`file://`).

The left `CAMPAIGN` explorer, centre tabbed document area, right `PROPERTIES`/`REFERENCES`, bottom `PROBLEMS`, menu strip and `▶ Play`/`⏹` match §I. The far-right `Flows` panel lists F1–F17 under plain-word titles. Click a flow to start a guided walkthrough. The banner sits above the editor (it does not cover the menu bar) and shows, in plain words:

- **Introduction** (before step 1): what you will make and why an author does it. Press `Start`.
- **Each step** (`Step n of m`): *what you are doing and why*, *what to click* (or *what to look at*), the editor terms met for the first time in that flow under *What's this?*, and *About the mock* where the prototype fakes part of the step. The next step opens with *Just happened*, so you see what the previous click did.
- **End card**: *What you should now see* and one *Question for you*; `Next flow` goes on to the next flow.

The element to use is outlined in gold. Clicking it advances. A *look* step highlights a panel to read; click it or press `Next`. Clicking anything else while a walkthrough is open shows a hint and changes nothing. `Back`, `Restart` and `Close` are always in the banner; after `Close` the editor can be explored freely.

The mock has no terrain tool, no drag, no typing and no Save, Load or Undo controls. Steps that need one say so under *About the mock* and show the finished result on the next screen.

The highlight targets are `data-wt` attributes. Menu buttons, drop-down items, explorer rows and `New…` buttons already had them. T102r added three, as attributes on existing panels (no layout change): `panel-properties`, `panel-references` and `panel-doc` (the middle panel), so a step can ask you to look at a panel. It also renamed the tab targets to `tab:<id>`, so a tab and its explorer row are no longer outlined together.

State is fake (no engine): "New campaign" adds explorer items, Play shows a text mock of the game view.

## Flow → screens and clicks

Flow ids F1–F17 are the ADR-0038 §F ids. The titles are plain-word titles for reviewers; the §17.2 item numbers are in ADR-0038 §F.

| Flow | Screens used | Clicks |
|---|---|---|
| F1 Start a new campaign | empty → campaign-created | Campaign ▸ New campaign…, ▶ Play |
| F2 Make a new area and add scenery | campaign-created → area-newtab → area-withprops | Worlds▸World▸New area (New…), Terrain/bake, place props |
| F3 Add the player, a villager and an enemy | area-withprops → creatures-created → references-area | move/make player start, Creatures▸New creature ×2, drag into area, References |
| F4 See where a creature's numbers come from | creatures-created → creature-stats | creature row, edit Might |
| F5 Write a conversation that starts a quest | creatures-created → dialogue-new → dialogue-texttoggle | Dialogue▸New dialogue, add choice/line, connect, Actions▸Start quest, text toggle |
| F6 Make a quest that finishes when the goblin dies | creatures-created → quest-new → event-page | Quests▸New quest, "Completes when…", on page |
| F7 Try the game with Play, then Stop | quest-new → play-start | ▶ Play, ⏹ |
| F8 Prepare the ground so the player can click to walk | area-withprops → play-navigate | bake navigation, ▶ Play, click ground |
| F9 Talk to the villager and start the quest | play-start → play-talk | click Elder, choose |
| F10 Set up a fight with the goblin | creatures-created → play-fight | Fights▸New fight, ▶ Play, attack |
| F11 See the quest complete | play-fight → play-questcomplete | (continues F10) |
| F12 Save, quit and load a game in progress | play-questcomplete → play-save | Save, Stop, Play, Load |
| F13 Run a game server with two players | play-save → play-two-clients | run crpg-server + two clients |
| F14 Make your own copy of the rules | campaign-created → rules-new | Rules▸New rules from template…, edit stat/roll table/Strike |
| F15 Change the rules for this campaign only | campaign-created → rules-override | Rules▸minimal-d6▸Change for this campaign… |
| F16 Bring in a 3D model for the goblin | campaign-created → asset-import-check → asset-import | Assets▸Import…, checks, Goblin▸Appearance |
| F17 Apply a plan from an agent | campaign-created → plan-applied | Campaign▸Apply plan…, Apply, Undo |

## Ambiguities found (findings for T101)

1. §F F2 names "Terrain tool (T063)" and "Place tool" but §I's menu strip (I1) has no Terrain/Place menu; only World. The prototype treats terrain bake and place as actions reachable from the area tab / explorer, not a top-level menu. T101 should decide where these tools live (menu vs. toolbar) and whether "Bake navigation" (F8) is a separate command or part of the terrain tool.
2. §F F3 says "right-click another placement → Make player start" but I1's menu strip lists no context-menu surface; the prototype exposes it as a row action. Whether right-click context menus exist is unspecified.
3. §I I1's menu strip has `Campaign ▾ Edit ▾ World ▾ Rules ▾ Test ▾ Package ▾ ▶ Play ⏹` but §F uses "Worlds ▸ World ▸ New area", "Creatures ▸ New creature", "Dialogue ▸ New dialogue", etc. (explorer-group "New…" per I2), and also "Campaign ▸ New…", "Rules ▸ New rules from template…", "Assets ▸ Import…". The prototype keeps both: explorer groups carry "New…" (I2) and the menus carry the campaign/rules/asset actions. T101 should confirm no duplicate "New area" in both World menu and explorer.
4. §F F10 allows a fight "that starts on approach (T108's mode rules)"; the prototype only models an explicit Fights ▸ New fight. The approach-trigger mode is not represented.
5. §F F12 is the game's save during Play, not the editor's file save; the prototype labels it as play-state save/load only, with no editor save affordance, matching the ADR's rejection of "§17.2-11 as an editor save".
6. §F F13 is not an M6 editor feature (Phase 10 multiplayer); the prototype shows a text mock only, per the brief.
7. §V V1 forbids ULIDs/paths/etc. in the GUI; the prototype shows short names only. Explorer rows show display text then short name (I2). Whether the short name is editable inline vs. in Properties is not specified (V2 says "editable"); prototype shows it as text.
8. §I I6 says Problems renders after every committed operation; the prototype shows a static empty/problems list per screen rather than recomputing, since there is no engine.
9. Rewriting the walkthrough (T102r) found more places where the mock has nothing to click: no Bake navigation command (F8; the Start tab stands in for it), no Actions menu or Text toggle (F5), no Save or Load (F12), no Undo control (F17), and the new fight does not appear in the explorer (F10). Each is stated in the step's *About the mock* line. T101 should say where Bake navigation and Undo live.

## Agent log
- 2026-11-21 (UTC) · opencode-go/beam-free + T102 · Built the single-file clickable prototype (index.html) covering F1–F17 with a data-driven screen/flow renderer, plus this README; findings above recorded for T101.
- 2026-11-21 (UTC) · claude-code/opus-5.5 (manager) + session 90 cleanup · Removed model-control tokens and a stray shell command that the T102 run left at the end of the previous entry, keeping that entry's text.
- 2026-11-22 (UTC) · claude-code/sonnet-5.5 (implementer) + T102r · Rewrote the prototype's guided walkthrough (flow introductions, three-part steps with plain-word terms, plain end-of-flow checks and a docked, readable banner) because the product owner could not follow the terse step text; targets were checked in a headless browser.
