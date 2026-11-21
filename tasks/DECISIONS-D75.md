# Decision D75 — a benchmark run that hits the Go quota part-way

Status: **draft, waiting on the user.**

## What happened
Session 90 started the 10 held-out `campaign-story` baseline runs (T100-E3). The OpenCode Go weekly limit ran out during the batch (reset in about 1 day 18 hours). Six runs were refused at the start; the scorer excluded them under T100-E2, as intended. Both DeepSeek V4 Pro runs had worked 55 and 72 steps when the limit hit. OpenCode then logged the quota error, scheduled a retry about 42 hours away, and opened a new step; the harness killed both at the 30-minute timeout. Because the last log line is that new step and not the error, `ends_in_service_fault` (`tools/agents/authorbench/authorbench_scoring.py:228`) returns false, so both runs count against the model as unfinished work (`valid 1`, `complete 0`). The two DeepSeek V4.1 Flash runs finished before the limit and are genuine: valid, played, but `complete` false because neither wrote the event graph the brief asks for (`minimum.graph = 1`).

## Question
Q1 — should a run whose log contains a Go quota error (`provider.go-limit`, HTTP 429 weekly or 5-hour) be excluded wherever the error appears, not only when it is the last line?
- **(a) Yes, erratum T100-E4 (recommended).** A quota stop is never transient: OpenCode's retry waits hours, so the run can only end in the timeout. Excluding it keeps a quota stop from being scored as the model's failure. Other service errors keep today's last-line rule, since those can recover. One function and its test change; the existing scores do not move (no earlier run hit a quota).
- **(b) No.** Keep the rule as it is and re-run any such run by hand. Cheaper now, but the next comparison run can fall into the same trap without anyone noticing.

Either way, the 8 refused or cut-off runs are re-run after the Go reset, and `docs/research/T100/baseline.*` is replaced only once all 10 have a real result.

## Agent log
- 2026-11-21 (UTC) · claude-code/opus-5.5 (manager) + session 90 · Drafted D75 after the Go weekly limit cut two baseline runs off mid-run and the scorer counted them against the model.
