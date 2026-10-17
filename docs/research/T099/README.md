# T099 research record: authoring-UX angles (M6-R)

The eight deep-pass reports for T099 are folded in here unchanged, except for inline `[check: ...]` markers. Each marker sits after a claim or citation that the citation check did not confirm. Check files live outside the repository, under `crpg-slots/research/`.

| File | Angle | Research model | Checker file | Confirmed / total | Not found | Other markers |
|---|---|---|---|---|---|---|
| [R8-deep.md](R8-deep.md) | Module-editor prior art | Cat Jam Free (`opencode/cat-jam-free`) | V8/V8.jsonl | 10 / 27 | 0 | 8 unavailable (7 markers); 9 paraphrase, not marked |
| [R9-deep.md](R9-deep.md) | Narrative tools: dialogue and quest authoring | Cat Jam (`opencode-go/cat-jam-free`, attempt 2) | V10/V10.jsonl | 19 / 19 | 0 | none |
| [R10-deep.md](R10-deep.md) | Authoring interfaces for AI agents | DeepSeek V4.1 Flash (`opencode-go/deepseek-v4.1-flash`) | V9/V9.jsonl | 29 / 29 | 0 | none |
| [R11-deep.md](R11-deep.md) | Usability methods for a one-person team | LongCat 2.5 Preview (`opencode-go/longcat-2.5-preview-free`, attempt 2) | V10/V10.jsonl | 26 / 36 | 6 (4 markers) | 3 unavailable (3 markers); 1 paraphrase, not marked |
| [R12-deep.md](R12-deep.md) | Event and trigger authoring for non-programmers | Cat Jam Free (`opencode/cat-jam-free`) | V8/V8.jsonl | 12 / 31 | 4 | 4 unavailable; 11 paraphrase, not marked |
| [R13-deep.md](R13-deep.md) | Ruleset authoring | Cat Jam Free (`opencode/cat-jam-free`) | V8/V8.jsonl | 15 / 23 | 1 | 4 unavailable; 3 paraphrase, not marked |
| [R14-deep.md](R14-deep.md) | Pause and time modes, single player and multiplayer | Muse Spark 1.3 Contributor (`opencode-go/muse-spark-1.3-contributor`) | V9/V9.jsonl | 44 / 61 | 0 | 17 unavailable (12 markers) |
| [R15-deep.md](R15-deep.md) | Custom content from module authors | Cat Jam (`opencode-go/cat-jam-free`, attempt 2) | V10/V10.jsonl | 14 / 17 | 0 | 3 paraphrase, not marked |

Counts are taken from the check files' rows. Several rows often share one marked line, so markers and rows do not match one to one. R10-deep's check file is `V9/V9.jsonl`, and R14-deep's is also there. The check files for R10 and R14 were found under V3–V9 and no other file named either report.

The handover's figures count differently and agree with these rows: there, a claim *held* if it was confirmed or paraphrased, out of the rows the checker could fetch (rows minus `unavailable`). R8 is (10 + 9) / (27 − 8) = 19/19, R11 (26 + 1) / (36 − 3) = 27/33, R12 (12 + 11) / (31 − 4) = 23/27, R13 (15 + 3) / (23 − 4) = 18/19, R14 44 / (61 − 17) = 44/44 and R15 (14 + 3) / 17 = 17/17. Research models are the last attempt in each report's `.agents/runs/<report>/` log.

## What "not found" means

A `not found` verdict means the checker could not locate the cited claim at the source. It does not mean the claim is wrong. Most of R11-deep's not-found rows are paywalled ACM and Taylor & Francis DOIs, so the checker could not read those pages. Treat them as unverified until someone with access reads the source. `unavailable` marks a source that could not be fetched. `paraphrase` rows are left unmarked: the checker found the claim's substance in the source, but not the wording.

## Agent log

- 2026-10-16 (UTC) · claude-code/bookkeeper + T099 fold-in · Recorded the eight citation-checked T099 reports and their check results so the M6-R authoring-UX research is traceable from the repository; markers flag unverified claims without changing the reports' text.
- 2026-10-16 (UTC) · claude-code (manager, Opus 5.5) + T099 fold-in check · Named each report's research model from its run log and replaced the handover-mismatch note with the reconciliation (held = confirmed + paraphrase, over fetchable rows), because both counts are correct.
