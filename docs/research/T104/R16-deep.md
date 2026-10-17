# R16 deep — Player UX for turn-based and RTwP CRPG combat

## Recommendations (ranked by expected impact)

1. Show roll transparency (hit chance %, dice/modifiers) before and after actions.
   - Why it applies: our data-driven rulesets must not hide math; players need trust in server-authoritative outcomes.
   - Evidence: https://en.wikipedia.org/wiki/Baldur%27s_Gate_3 — "Players can see the percentage-chance that an attack will hit before making it; this value is impacted by things like terrain."
   - Strength: practitioner report | Effort: medium

2. Make armor/HP damage deterministic enough for planning, with small RNG "spice".
   - Why it applies: supports both TB and RTwP planning across arbitrary rulesets.
   - Evidence: https://www.gamedeveloper.com/design/designing-drama-into-the-turn-based-combat-of-i-divinity-original-sin-2-i- — "Rather than absorbing a proportion of incoming damage, armor completely negates it."
   - Strength: practitioner report | Effort: medium

3. Provide a free-floating camera (isometric to third-person) plus tactical zoom.
   - Why it applies: one camera rig must serve exploration and tactical combat in both modes.
   - Evidence: https://en.wikipedia.org/wiki/Baldur%27s_Gate_3 — "It has a free-floating camera, with players able to adjust the perspective from top-down isometric to third person."
   - Strength: practitioner report | Effort: large

4. Do not rely on color alone; ship custom colorblind themes, not just OS high-contrast.
   - Why it applies: accessibility must work in our Godot client without breaking aesthetics.
   - Evidence: https://arxiv.org/html/2401.10357v2 — "operating-system-wide high contrast mode can reduce both aesthetics and functionality."
   - Strength: measured study | Effort: small

5. Display initiative/turn order explicitly (dice roll + modifiers).
   - Why it applies: server-authoritative multiplayer needs a shared, readable turn queue.
   - Evidence: https://en.wikipedia.org/wiki/Baldur%27s_Gate_3 — "A d4 roll, called an initiative roll, determines the order in which each character acts."
   - Strength: practitioner report | Effort: small

6. Pace AI threat so players can react; avoid one-shot focus fire.
   - Why it applies: keeps combat readable and fair in co-op.
   - Evidence: https://www.gamedeveloper.com/design/designing-drama-into-the-turn-based-combat-of-i-divinity-original-sin-2-i- — "for AI it always makes sense to pile on one person and just murder them completely, but for the player it really just sucks"
   - Strength: practitioner report | Effort: medium

7. Offer optional auto-pause triggers in RTwP (end of round, low HP, enemy spotted).
   - Why it applies: lets players manage complex rulesets without reflex demands.
   - Evidence: https://en.wikipedia.org/wiki/Pillars_of_Eternity — "the combat's customizability in the game, including the ability to change the difficulty and set options for auto-pausing."
   - Strength: practitioner report | Effort: small

8. Show hit chance and health loss preview before committing an action.
   - Why it applies: reduces misplays in multiplayer and supports learning.
   - Evidence: https://en.wikipedia.org/wiki/XCOM_2 — "The game will inform the players of the possibility of landing a successful shot and the amount of health the enemy will lose."
   - Strength: practitioner report | Effort: small

9. Follow WCAG 1.4.1: never use color as the only visual means of conveying info.
   - Why it applies: baseline for all HUD elements across rulesets.
   - Evidence: https://www.w3.org/WAI/WCAG21/Understanding/use-of-color.html — "Color is not used as the only visual means of conveying information, indicating an action, prompting a response, or distinguishing a visual element."
   - Strength: vendor doc | Effort: small

## Comparison table (one row per game)

| Game | Camera | Action selection | Targeting preview | Turn-order display | Roll transparency | Log detail | URL |
|---|---|---|---|---|---|---|---|
| Baldur's Gate 3 | Free-floating; isometric to third-person | Actions/bonus actions/movement | Hit-chance % before attack | d4 initiative roll order | Hit-chance % visible | Not stated in fetched sources | https://en.wikipedia.org/wiki/Baldur%27s_Gate_3 |
| Solasta | Not stated in fetched sources | Not stated in fetched sources | Not stated in fetched sources | Attacks of opportunity noted | "dice decide your destiny" (marketing) | Not stated in fetched sources | https://solastacrownofthemagister.fandom.com/wiki/Solasta:_Crown_of_the_Magister |
| Pathfinder: Kingmaker | Isometric | Pausable RT or turn-based toggle | Not stated in fetched sources | Not stated in fetched sources | Not stated in fetched sources | Not stated in fetched sources | https://en.wikipedia.org/wiki/Pathfinder:_Kingmaker |
| Pathfinder: Wrath of the Righteous | Isometric | Turn-based or RT via setting | Not stated in fetched sources | Not stated in fetched sources | Not stated in fetched sources | Not stated in fetched sources | https://en.wikipedia.org/wiki/Pathfinder:_Wrath_of_the_Righteous |
| Pillars of Eternity I | Fixed isometric | RTwP with auto-pause options | Bestiary stats shown | Not stated in fetched sources | Not stated in fetched sources | Not stated in fetched sources | https://en.wikipedia.org/wiki/Pillars_of_Eternity |
| Pillars of Eternity II: Deadfire | Isometric | RTwP or turn-based (added patch) | Not stated in fetched sources | Not stated in fetched sources | Not stated in fetched sources | Not stated in fetched sources | https://en.wikipedia.org/wiki/Pillars_of_Eternity_II:_Deadfire |
| Divinity: Original Sin 2 | Isometric | Action points per turn | Armor/HP bars; damage variability | Not stated in fetched sources | Armor/HP bars; small RNG damage | Not stated in fetched sources | https://en.wikipedia.org/wiki/Divinity:_Original_Sin_II |
| Wasteland 3 | Isometric | Turn-based | Not stated in fetched sources | Not stated in fetched sources | Not stated in fetched sources | Not stated in fetched sources | https://en.wikipedia.org/wiki/Wasteland_3 |
| XCOM 2 | Top-down | Turn-based | Shot success % and health loss shown | Not stated in fetched sources | Shot success % shown | Not stated in fetched sources | https://en.wikipedia.org/wiki/XCOM_2 |

## What the research literature says

- Color vision deficiency affects over 300 million people; color-only cues reduce accessibility. https://arxiv.org/abs/2401.10357v2
- OS high-contrast mode reduced both aesthetics and functionality in simulated CVD screenshots (Pearson r=0.74 aesthetics/functionality). https://arxiv.org/html/2401.10357v2
- Color use in GUIs involves contradictions (customizability vs. confusion). https://arxiv.org/abs/1404.6773v1
- Usability principles can be implemented into a game UI/HUD framework. https://eprints.hud.ac.uk/id/eprint/17809/
- Visual-impairment accessibility needs user-centered design. https://vtechworks.lib.vt.edu/items/624412d8-b1d2-4a65-9f2f-abe23013e5da/full
- Adaptive inclusive systems should start from disability needs. https://tud.qucosa.de/id/qucosa%3A31984
- Real-time-with-pause and turn-based modes can be offered as a setting. https://en.wikipedia.org/wiki/Timekeeping_in_games

## Contradictions (both sides cited)

- Aesthetics vs. functionality: Karvonen (2000) argues they conflict, while Kurosu & Kashimura (1995) found positive correlation; arXiv CVD study found r=0.74. https://arxiv.org/html/2401.10357v2
- High-contrast mode is intended to aid accessibility but reduced functionality/aesthetics in the CVD simulation study. https://arxiv.org/html/2401.10357v2
- Deterministic armor aids planning vs. small damage variability keeps combat "spicy". https://www.gamedeveloper.com/design/designing-drama-into-the-turn-based-combat-of-i-divinity-original-sin-2-i-
- Merciless AI focus-fire is optimal vs. dramatic pacing that lets players react. https://www.gamedeveloper.com/design/designing-drama-into-the-turn-based-combat-of-i-divinity-original-sin-2-i-

## Not applicable

- 3DUI/VR thesis findings (head tracking, stereoscopic 3D) do not apply to our 2D isometric HUD. https://stars.library.ucf.edu/etd/1143
- XCOM time-pressure/concealment design is for tactics games, not narrative CRPG pacing. https://en.wikipedia.org/wiki/XCOM_2
- OS high-contrast mode is not a substitute for in-game colorblind themes. https://arxiv.org/html/2401.10357v2

## Agent log

- 2026-10-17 (UTC) · claude-code (manager, Opus 5.5) + T104 fold-in · Keeps the R16 deep pass on combat player UX as the T104 record; the V11 check confirmed or found the substance of all 32 citations, so it carries no markers.
