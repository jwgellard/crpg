# Decision D73 — the authoring benchmark (T100): runs cut short by a service fault

Status: **answered, 2026-11-21.** D73a (b).

The authoring benchmark (T100, D72) hands each AI model a task, lets it work,
and then scores what it wrote. The scorer already leaves out a run that never
started (the model service refused before the model took a single step), so a
service outage does not count against the model. It does **not** leave out a
run that started and was then cut off by the service itself: in session 86 five
runs ended mid-work with the OpenCode service's own error ("Failed to execute
statement", all five at the same moment, and one "HTTP 500"). Those five were
counted as the model failing the task, although the model did nothing wrong.
Session 87 re-ran that task by hand, which works but has to be noticed each
time.

## D73a — should a run cut off by a service fault count as a model failure?

- (a) **Keep it: count it as a failure.** Simple, but the baseline and every
  later comparison can be skewed by an outage on the day, and the manager has
  to spot and re-run such runs by hand.
- (b) **Leave it out, like a run that never started (recommended).** A run
  whose last recorded event is a service error (`Failed to execute statement`,
  or an HTTP 5xx status) is marked "excluded", shown in the table's excluded
  count, and left out of the averages. A model error, a wrong file, a failed
  `crpgc` command or a timeout still counts against the model. This is a small
  erratum to the T100 contract and a few lines in the scorer and its tests
  (OpenCode worker, Python only, no engine code, no new library).

**Recommend (b).** Under (a) nothing changes. Under (b) the scoring of runs
already made may change slightly when the baseline is re-scored (re-scoring
needs no new runs).

**Answer (user, 2026-11-21): (b).**

## Agent log
- 2026-11-21 (UTC) · claude-code/opus-5.5 (manager) + session 88 · Drafted D73a from the session 86–87 baseline, where five runs cut off by the OpenCode service were counted as model failures, with a recommendation.
- 2026-11-21 (UTC) · claude-code/opus-5.5 (manager) + D73 answer · Recorded the user's answer (b): runs cut off by a service fault are excluded; the scorer change is a T100 erratum still to be written.
