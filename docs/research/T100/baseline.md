# T100 authoring benchmark — baseline

The first measurement of the authoring benchmark (`tasks/T100.md`, D72) on today's tools: `crpgc` at `d11cc6d` and the 23 schemas, design `baseline`. Five Go models × five briefs × 2 repeats = 50 runs, made in sessions 86–88 at `--par 2`/`--par 3`. campaign-extend was re-run in full because the session 86 attempt had five infrastructure stops; ruleset-small's one DeepSeek V4 Pro timeout (30 min) counts against the model. Machine-readable scores: `baseline.scores.json` (re-scoring gave the same bytes). Re-scored under erratum T100-E1 on 2026-11-21: the `errors` column now counts failed `crpgc` calls (the first scoring read no `shell` call and showed 0 everywhere); the other columns did not change. T101's design is re-measured against this table. Re-scored under erratum T100-E2 on 2026-11-21 (runs cut off by a service fault are excluded): identical bytes, 0 excluded, because the session 86 faulted runs had already been re-run.

```text
SCORED baseline: 50 runs, 45 valid, 45 playable, 0 excluded (smoke ok)
| campaign-extend | opencode-go/claude-haiku-5-5 | n=2 | valid 2 | complete 2 | playable 2 | finished 2 | steps 11.0 | errors 1.0 |
| campaign-extend | opencode-go/deepseek-v4-pro | n=2 | valid 2 | complete 2 | playable 2 | finished 2 | steps 21.0 | errors 1.0 |
| campaign-extend | opencode-go/deepseek-v4.1-flash | n=2 | valid 2 | complete 2 | playable 2 | finished 2 | steps 16.5 | errors 1.0 |
| campaign-extend | opencode-go/glm-5.3 | n=2 | valid 2 | complete 2 | playable 2 | finished 2 | steps 22.0 | errors 0.5 |
| campaign-extend | opencode-go/muse-spark-1.3-contributor | n=2 | valid 2 | complete 2 | playable 2 | finished 2 | steps 16.5 | errors 0.5 |
| campaign-small | opencode-go/claude-haiku-5-5 | n=2 | valid 2 | complete 2 | playable 2 | finished 2 | steps 36.5 | errors 17.0 |
| campaign-small | opencode-go/deepseek-v4-pro | n=2 | valid 1 | complete 1 | playable 1 | finished 1 | steps 92.5 | errors 5.5 |
| campaign-small | opencode-go/deepseek-v4.1-flash | n=2 | valid 2 | complete 2 | playable 2 | finished 2 | steps 63.0 | errors 2.5 |
| campaign-small | opencode-go/glm-5.3 | n=2 | valid 2 | complete 2 | playable 2 | finished 2 | steps 102.0 | errors 25.5 |
| campaign-small | opencode-go/muse-spark-1.3-contributor | n=2 | valid 2 | complete 2 | playable 2 | finished 2 | steps 64.5 | errors 0.0 |
| campaign-medium | opencode-go/claude-haiku-5-5 | n=2 | valid 1 | complete 2 | playable 1 | finished 1 | steps 41.0 | errors 6.0 |
| campaign-medium | opencode-go/deepseek-v4-pro | n=2 | valid 1 | complete 1 | playable 1 | finished 1 | steps 74.5 | errors 4.5 |
| campaign-medium | opencode-go/deepseek-v4.1-flash | n=2 | valid 2 | complete 2 | playable 2 | finished 2 | steps 105.5 | errors 23.0 |
| campaign-medium | opencode-go/glm-5.3 | n=2 | valid 2 | complete 2 | playable 2 | finished 2 | steps 146.5 | errors 9.0 |
| campaign-medium | opencode-go/muse-spark-1.3-contributor | n=2 | valid 1 | complete 2 | playable 1 | finished 1 | steps 109.5 | errors 0.5 |
| campaign-large | opencode-go/claude-haiku-5-5 | n=2 | valid 2 | complete 2 | playable 2 | finished 1 | steps 54.5 | errors 26.0 |
| campaign-large | opencode-go/deepseek-v4-pro | n=2 | valid 2 | complete 2 | playable 2 | finished 2 | steps 129.0 | errors 12.5 |
| campaign-large | opencode-go/deepseek-v4.1-flash | n=2 | valid 2 | complete 2 | playable 2 | finished 2 | steps 76.0 | errors 13.5 |
| campaign-large | opencode-go/glm-5.3 | n=2 | valid 2 | complete 2 | playable 2 | finished 0 | steps 165.0 | errors 10.0 |
| campaign-large | opencode-go/muse-spark-1.3-contributor | n=2 | valid 2 | complete 2 | playable 2 | finished 2 | steps 87.0 | errors 1.0 |
| ruleset-small | opencode-go/claude-haiku-5-5 | n=2 | valid 2 | complete 2 | playable 2 | finished 2 | steps 45.0 | errors 22.5 |
| ruleset-small | opencode-go/deepseek-v4-pro | n=2 | valid 1 | complete 2 | playable 1 | finished 1 | steps 79.0 | errors 3.0 |
| ruleset-small | opencode-go/deepseek-v4.1-flash | n=2 | valid 2 | complete 2 | playable 2 | finished 2 | steps 42.0 | errors 5.5 |
| ruleset-small | opencode-go/glm-5.3 | n=2 | valid 2 | complete 2 | playable 2 | finished 2 | steps 86.0 | errors 8.5 |
| ruleset-small | opencode-go/muse-spark-1.3-contributor | n=2 | valid 2 | complete 2 | playable 2 | finished 2 | steps 69.0 | errors 4.0 |
```

## Agent log
- 2026-11-21 (UTC) · claude-code/opus-5.5 (manager) + T100 baseline · Recorded the completed 50-run baseline and its scores, so T101's authoring design can be measured against it.
- 2026-11-21 (UTC) · claude-code/opus-5.5 (manager) + T100-E1 re-score · Replaced the table and scores with the re-scored ones, because the first scoring counted no `crpgc` call and showed every `errors` cell as 0.
- 2026-11-21 (UTC) · opencode-go/muse-spark-1.3-contributor + S89-bookkeeping · Noted the T100-E2 re-score (identical bytes, 0 excluded), so the baseline records the service-fault exclusion rule.
