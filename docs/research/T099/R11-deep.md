# R11-deep — Usability methods for a one-person team (deep pass)

## Recommendations (ranked by expected impact)

1. **Run many tiny think-aloud rounds (≈5 testers) from the prototype on.** *Confirmed from pass 1.* Why: the plan is a clickable prototype then gates; a single early round surfaces most problems cheaply. Evidence: https://www.nngroup.com/articles/why-you-only-need-to-test-with-5-users/ — "After the first study with five participants has found 85% of the usability problems." Strength: measured study. Effort: small.

2. **Run a heuristic evaluation (3–5 evaluators) before each user test.** *Confirmed from pass 1.* Why: stretches a tiny budget and trains UX instincts. Evidence: https://www.nngroup.com/articles/how-to-conduct-a-heuristic-evaluation/ — "heuristic evaluations cannot replace user research." and "three to five people should independently evaluate." Strength: practitioner report. Effort: small.

3. **Cognitive-walkthrough the first-run and novel flows.** *Confirmed from pass 1.* Why: our audience are first-time module authors with no mental model of the JSON. Evidence: https://www.nngroup.com/articles/cognitive-walkthroughs/ — "evaluators discuss 4 key questions (analysis criteria)" and "most effective for systems with complex, new, or unfamiliar workflows." Strength: practitioner report. Effort: small.

4. **Treat first-use empty states as onboarding, not absence.** *Confirmed from pass 1.* Why: the gate requires someone building from a blank campaign. Evidence: https://v10.carbondesignsystem.com/patterns/empty-states-pattern/ — "Empty states are a pivotal point in the user journey. They are opportunities to build rapport, drive engagement, educate." and "Starter content can provide the opportunity for users to dive in and learn about primary features." Strength: vendor doc. Effort: small. [check: not found — V10]

5. **Recruit volunteer modders with real informed consent and opt-in only.** *Confirmed from pass 1.* Why: community testers are not employees and must be credited, not harvested. Evidence: https://www.nngroup.com/articles/informed-consent/ — "Informed consent is an exchange of information in which both of the following must occur." and https://www.soulframe.com/en/participation-agreement — "You participate in the Test Program strictly on a volunteer basis and without any form of compensation." Strength: vendor doc. Effort: small.

6. **Use AI agents as cheap early screeners only, never as proof of human usability.** *Corrects pass 1.* Why: our free model can walk CLI/schema flows, but LLM "users" are biased proxies. Evidence: https://arxiv.org/html/2502.12561v3 — "participants praised the innovation of our system but also expressed concerns about the future of UX study with LLM Agents" and "LLM Agents are not to replace human participants." and https://arxiv.org/html/2505.22981 — "12.8 simulated agents are as useful as one locally recruited human" and "aggregated simulations produce representative and actionable insights comparable to real users." Strength: measured study. Effort: medium.

7. **Keep a quantitative gate with real tasks/metrics before release.** *Confirmed from pass 1.* Why: "someone who has never seen the JSON builds a small campaign" needs numbers, not vibes. Evidence: https://www.nngroup.com/articles/how-many-test-users/ — "Test at least 20 users to get statistically significant numbers." and https://measuringu.com/task-completion/ — "An analysis of almost 1200 usability tasks shows that the average task-completion rate is 78%." Strength: measured study. Effort: medium.

8. **Design the prototype walkthrough around workflows, not features.** *New.* Why: tool UX research shows the biggest wins come from fixing broken workflows, not adding features. Evidence: https://thetoolsmiths.org/codex/gdc/roundtable/tools_design/2019/day-1 — "Stop looking at tools and start looking at workflows (multiple studios are doing this)" and "finding broken workflows." and https://www.gamedeveloper.com/design/don-t-fear-the-hammer-the-importance-of-ux-in-editors-tools — "It's the flow of the tool. The UX." and "One person solving one of these issues in a week can save 20 tool users an hour a day." Strength: practitioner report. Effort: small.

9. **Recruit testers who are fans of the genre but not personally invested in your project.** *New.* Why: invested testers hide problems; engaged outsiders surface them. Evidence: https://www.gamedeveloper.com/production/post-mortem-black-mesa-hazard-course — "As fans of Half-Life and Black Mesa, but not people who were personally invested in any particular part of our vision, our testers kept us reined in and focused." Strength: practitioner report. Effort: small.

10. **Use structured community testing workflows (application forms, feedback channels, clear phases).** *New.* Why: modding communities have solved volunteer testing at scale. Evidence: https://stepmodifications.org/wiki/STEP:Guide-Community_Mod_Testing — "any member may contribute evidence" and "If the mod receives a positive review and is advocated by at least two staff, it should be tagged ACCEPTED." and https://www.nexusmods.com/news/14568 — "Anyone who is interested can apply to become a tester via the Google form linked below." Strength: community doc. Effort: small.

11. **Know a paper/clickable prototype cannot answer look/feel/efficiency.** *Confirmed from pass 1.* Why: don't sign off responsiveness from it. Evidence: https://web.mit.edu/6.813/www/sp16/classes/10-prototyping/ — "paper prototypes don't reveal every usability problem, because they are low-fidelity in several dimensions." and "Users are more deliberate with a paper prototype; they don't explore or thrash as much." Strength: measured study. Effort: small.

12. **Use classic think-aloud (Level 1–2 verbalizations) to avoid altering behaviour.** *New.* Why: the deep pass found that relaxed think-aloud changes user behaviour; classic does not. Evidence: https://doi.org/10.1145/3665327 — "most studies find that classic thinking aloud does not alter performance, except by prolonging it" and "many studies document that relaxed thinking aloud alters behaviour and performance." and https://doi.org/10.1080/01449290701773842 — "classic thinking aloud has little or no effect on behaviour apart from prolonging tasks, relaxed thinking aloud affects behaviour in multiple ways." Strength: measured study. Effort: small. [check: not found — V10]

## What the research literature says

- **Nielsen & Landauer (INTERCHI '93)** — The original Poisson model behind "5 users." For 11 studies, "the detection of usability problems as a function of number of users tested or heuristic evaluators employed is well modeled as a Poisson process." https://dl.acm.org/doi/10.1145/169059.169166 [check: unavailable — V10]

- **Spool & Schroeder (CHI '01)** — The famous critique. 49 users on four e-commerce sites found "considerably more than five users" needed. "On sites I and II, taking L = 0.1 we had found approximately 35% of the problems after the first five users." https://dl.acm.org/doi/10.1145/634067.634236 [check: unavailable — V10]

- **Virzi, Sokolov & Karis (CHI '96)** — Low-fidelity prototypes find the same problems as high-fidelity. "substantially the same sets of usability problems were found in the low- and high-fidelity conditions." https://www.chi1996.chistatic.hosting.acm.org/proceedings/papers/Virzi/RAVtext.html

- **Hertzum et al. (2009)** — Think-aloud behaviour effects. "classic thinking aloud has little or no effect on behaviour apart from prolonging tasks, relaxed thinking aloud affects behaviour in multiple ways." https://doi.org/10.1080/01449290701773842 [check: not found — V10]

- **Hertzum (2024 meta-analysis)** — 29 studies comparing concurrent vs retrospective think-aloud. "most studies find that classic thinking aloud does not alter performance, except by prolonging it." https://doi.org/10.1145/3665327 [check: not found — V10]

- **UXAgent (CHI EA '25)** — LLM agents as usability testers. 5 UX researchers found simulated data "very helpful" but "not like real humans." https://arxiv.org/html/2502.12561v3

- **CSUA (2025)** — 240 simulated players matched human findings. "12.8 simulated agents are as useful as one locally recruited human." https://arxiv.org/html/2505.22981

- **SimUser (CHI '24)** — LLM-generated usability feedback. "SimUser could generate heuristic usability feedback with the similarity varying from 35.7% to 100%." https://dl.acm.org/doi/full/10.1145/3613904.3642481 [check: unavailable — V10]

- **Georgsson (2020)** — Cognitive walkthrough vs think-aloud. "A total of 12 evaluators, six per method, detected 18 usability problems with the CW. Twenty were found with the TA." https://www.ejbi.org/scholarly-articles/nasa-rtlx-as-a-novel-assessment-tool-for-determining-cognitive-load-and-user-acceptance-of-expert-and-userbased-usabilityevaluatio-5988.html

- **GDC Lightbown (2013)** — "The games industry needs to make the user experience of tools a priority." https://gdcvault.com/play/1019400/The-User-Experience-of-Game

- **Toolsmiths Roundtable (2019)** — "Stop looking at tools and start looking at workflows." https://thetoolsmiths.org/codex/gdc/roundtable/tools_design/2019/day-1

- **Black Mesa post-mortem** — "As fans of Half-Life and Black Mesa, but not people who were personally invested in any particular part of our vision, our testers kept us reined in and focused." https://www.gamedeveloper.com/production/post-mortem-black-mesa-hazard-course

## Pass-1 claims that did not hold

- **Pass 1 cited alphaxiv.org/abs/2502.12561 as evidence for AI testers.** The actual arXiv paper (2502.12561) is a CHI EA '25 paper with a 5-researcher heuristic evaluation, not a large-scale measured study of simulated vs human behaviour. The claim "researchers valued the system's ability to generate large-scale preliminary data" is supported but the framing was imprecise.
- **Pass 1 cited arxiv.org/pdf/2603.11245v2 and aclanthology.org/2026.acl-long.2192 for LLM user simulator limitations.** These were not fetched in this run. The CSUA paper (arxiv.org/html/2505.22981) provides stronger evidence: simulated agents plateau at ~90% coverage of human findings, and individual agents are imperfect.
- **Pass 1 did not distinguish classic vs relaxed think-aloud.** The deep pass found this matters: classic think-aloud (Level 1–2) does not alter behaviour; relaxed think-aloud (Level 3) does. This changes how we should instruct testers.
- **Pass 1 did not cover the Toolsmiths/Lightbown practitioner literature.** This is the most relevant practitioner source for game tool UX and should inform our editor design.

## Contradictions

- **"5 users find 85%" (Nielsen) vs Spool & Schroeder.** Spool found "approximately 35% of the problems after the first five users" on e-commerce sites. Both sides cited. Resolution: 5 is a default for homogeneous tasks; complex novel tools may need more.
- **Low-fidelity prototypes find same problems (Virzi '96) vs MIT 6.813.** MIT notes paper prototypes miss look/feel/efficiency. Both true: same *usability* problems, different *classes* of problems.
- **Classic think-aloud doesn't alter behaviour (Hertzum 2009/2024) vs NNGroup's "robust" claim.** NNGroup says "Most people are poor facilitators... you'll still get reasonably good findings." Hertzum shows the mechanism: classic doesn't alter, relaxed does.
- **AI testers: UXAgent says "not replace humans" vs CSUA says "12.8 agents = 1 human."** Both true: individual agents are imperfect, but aggregated at scale they approach human coverage (~90%).
- **Empty states as onboarding (Carbon/Workday) vs NNGroup's "skip onboarding when possible."** Both agree: starter content beats blank screens, but forced tours hurt.

## Not applicable

- Eyetracking (39 users) and card sorting (≈15) are overkill for our budget.
- Quantitative benchmarks belong at the pre-release gate, not every checkpoint.
- Pixel-perfect visual polish before flow validation.
- Telemetry/harvesting tester data; recruit with clear opt-in instead.
- Using employees/volunteer insiders as the final gate testers (last resort; they carry company/product knowledge).

## Agent log

- 2026-10-16 (UTC) · claude-code/bookkeeper + T099 fold-in · Keeps the deep pass's usability-method findings as the T099 record; markers flag claims the V10 check could not match, several of them paywalled sources.
