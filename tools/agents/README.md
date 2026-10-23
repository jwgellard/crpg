# tools/agents — agent dispatch scripts

Scripts the manager uses to start workers and check their results. Every
script prints a verdict first, keeps output to at most 40 lines (`--full`
lifts the cap, `--json` prints one object instead), and exits 0 (verified),
1 (a check failed) or 2 (usage or infrastructure error). See
`docs/AGENT_TEAM_PLAN.md` §5.1 and `docs/agents/MODELS.md` §1.

## oc.py — the OpenCode dispatcher (A0-3)

Starts OpenCode workers safely on Windows and Linux, runs the verifier on
their result, and prints a verdict-first summary. Stdlib only, Python 3.11+.

```text
python tools/agents/oc.py run  --task ID --role ROLE --model MODEL [--slot K]
                               [--timeout SEC] [--verify-timeout SEC]
                               [--variant V] [--brief PATH]
                               [--notes PATH] [--slug SLUG] [--base REF]
                               [--fresh] [--dry-run] [--json] [--full]
python tools/agents/oc.py wave SPEC [SPEC ...] [--timeout SEC]
                                [--verify-timeout SEC] [--base REF] [--json]
python tools/agents/oc.py ask  --role scout --model MODEL [--diff REF]
                               [--base BASE] [--timeout SEC] [--json] QUESTION
python tools/agents/oc.py record --task ID --role ROLE --model MODEL --outcome OUTCOME
                               [--attempt N] [--note TEXT]
python tools/agents/oc.py stats [--role ROLE] [--since YYYY-MM-DD] [--json]
python tools/agents/oc.py spend [--json]
```

`SPEC` is `ID:ROLE:MODEL`; `ROLE` is `implement`, `tests`, `review`,
`scout` or `grunt`, and the worker runs as the `crpg-<role>` agent.
`OUTCOME` is `review-ready`, `review-fix`, `review-reject`, `merged` or
`reverted`. Slots are warm git worktrees under `$CRPG_SLOTS` (default
`<repo>/../crpg-slots`); without `--slot`, the lowest-numbered free slot
wins. Every `run`, `ask` and `wave` member leaves a result record under
`<repo>/.agents/runs/<ID>/` (the model scorecard `stats` ranks) and an
event log beside it. The attempt number counts earlier runs of the same
task ID and the same `--slug` only: `--slug draft-a` and `--slug draft-b`
under T136b both start at attempt 1, while rerunning `draft-a` is attempt
2, and a run with no slug counts only slug-less runs. A slug run's files
are named `<attempt>-<slug>-<model>.{jsonl,result.json,report.md}`; a
slug-less run keeps `<attempt>-<model>…`, so `stats` reads both.

Environment: `$CRPG_REPO` (repo root, default: the main worktree),
`$CRPG_SLOTS`, `$CRPG_OPENCODE` (executable; a `.py` path runs under
`sys.executable`), `$CRPG_VERIFY` (verifier path), `$CRPG_OPENCODE_LOG`
(OpenCode's own log, for the idle watchdog), `$CRPG_OC_START_GAP` (worker
start spacing, default 4 s) and `$CRPG_OC_LOCK_WAIT` (locked retry wait,
default 5 s). The executable is `$CRPG_OPENCODE`, else the OpenCode 2.x
path (`%APPDATA%\npm\node_modules\@opencode\cli\bin\opencode.exe`), else
the 1.x path (`...\opencode-ai\bin\opencode.exe`), else `opencode` on
`PATH`. The major version is probed once per process with `<exe>
--version`; 2.x runs as `run --standalone --agent crpg-<role> -m
<model>[#<variant>] --format json <prompt>` with no `--dir` (the slot is
the process cwd plus `PWD`), while 1.x keeps `run --agent crpg-<role>
-m <model> [--variant V] --dir <slot> --format json <prompt>`. The stall
retry uses variant `low` in whichever form applies, and `--dry-run`
prints the command for the detected version.

`wave` caps concurrency per the manager's routing rules
(`docs/agents/MODELS.md` §4): at most one run per free Zen model
(`opencode/…-free`), at most three free Zen runs in total, and at most
three runs per model of any other kind. Members over a cap wait for a
free place in spec order; they are not refused. A model ID that moved is
refused (exit 2 `usage`, naming the new ID; one constant,
`RENAMED_MODELS`): `opencode/longcat-2.5-preview-free` →
`opencode-go/longcat-2.5-preview-free`, and Cat Jam, which became Beam
Free after the OpenCode 2.x upgrade, `opencode/cat-jam-free`,
`opencode-go/cat-jam-free` and `opencode/beam-free` →
`opencode-go/beam-free`.

`spend` prints the Go spend over a rolling 5-hour window and a rolling 7-day
window, summed from the `cost` field of the result records under
`.agents/runs/` for models whose ID starts with `opencode-go/` and whose
`started_utc` is inside the window (a missing stamp or non-numeric cost counts
as $0). The first line is the verdict (`SPEND`, totals against the caps),
followed by each window's spend per model, largest first; `--json` prints the
same data.

The same numbers guard `run`, `ask` and every `wave` member before any slot is
prepared or a process starts. A Go start is refused (exit 2, cause `spend`)
when the window total plus the model's reservation would exceed its cap, or
when a scarce model (`kimi-k3`, `gemini-4-argon-contributor`) already started
within 5 hours — at most one such run per window, per `docs/agents/MODELS.md`
§1 item 16. The refusal prints the spend line and one line naming the rule,
the totals, the cap and the override, and leaves the usual cost-0 result
record. `run --dry-run` applies the guard too. Caps are `GO_CAP_5H` = $4.50
and `GO_CAP_WEEK` = $12.00, overridable with `$CRPG_GO_CAP_5H` and
`$CRPG_GO_CAP_WEEK`; `CRPG_GO_SPEND_OVERRIDE=1` skips the guard and the
summary says `spend guard overridden`.

`ask --diff REF` gives a reviewer a slot branch's diff without leaving the
main checkout. Before OpenCode starts it runs `git diff <BASE>...<REF>` in the
repository root (branches are shared by every worktree, so a slot branch
resolves from the main checkout), writes the output to
`<repo>/.agents/runs/ask/<attempt>-<model>.diff` (the log's stem with a
`.diff` suffix), and then appends one blank line and exactly:

    The diff of `<REF>` against `<BASE>` is in `<path>`; read it there.

where `<path>` is that file's path relative to the repository root, with
forward slashes. `--base` defaults to `master` and is only meaningful with
`--diff`; `--base` alone exits 2 with a usage error. An unresolved `REF` or
`BASE` exits 2 `bad-ref`; an empty diff exits 2 `empty-diff`. Both happen
before any OpenCode process starts, still write the cost-0 result record, and
neither counts against the model. The record and the `--json` payload carry
the diff path under `diff` (absent without `--diff`). Without `--diff`,
`ask` behaves exactly as before.

Briefs: `run` takes `--brief`, else `<repo>/.agents/briefs/<ID>.md`
when filed, else generates one via `brief.py`; `wave` resolves each
member the same way (it has no `--brief` of its own). A brief that
still contains the manager template's `<MANAGER:` placeholder is
refused (exit 2 `no-brief`), as is a missing brief or generator.
`wave` reads each member brief's `## Files you own` bullets and refuses
(exit 2 `usage`) two members owning paths under the same
`crates/<name>/`, since each crate takes one task at a time.

`--base REF` (default `origin/master`) is both the ref each slot's task
branch starts from and the ref handed to `verify_task.py` as `--base`, so a
run prepared from a non-master base is not reported as a pile of
out-of-scope base commits. Its first path segment names a remote only when
`git remote` in the slot lists it, so `origin/mgr/session-10-batch` fetches
`mgr/session-10-batch` from `origin` while `mgr/session-10-batch` is a
local ref and nothing is fetched. When a slot is not on the task branch,
`run` checks the branch out, or creates it from `--base` when it does not
exist yet; a slot with uncommitted changes is refused (`branch`). `wave`
passes its own `--base` to every member.

Agents: OpenCode reads `opencode.json` and `.opencode/agents/crpg-<role>.md`
from the slot. When the slot has no `crpg-<role>.md` (a base that predates
A0-2), `run` copies `opencode.json` and every missing file under
`.opencode/agents/` from the repository root into the slot — never
overwriting a file the slot already has — and lists the copies in the slot
worktree's `info/exclude` (`git rev-parse --git-path info/exclude`), so
`verify_task.py`'s scope check does not count them as the worker's changes.
If the repository root has no declaration either, the run exits 2 `no-agent`
before OpenCode starts; an OpenCode event reading `Agent not found`
classifies as the same cause.

Exit-2 causes: `fetch`, `no-brief`, `no-verify`, `no-slot`, `slot-busy`,
`no-exe`, `no-agent`, `branch`, `bad-ref`, `empty-diff`, `locked`, `402`,
`quota`, `spend`, `verify-timeout`, `timeout`, `hang`, `no-edit`, `model`,
`verify-error`, `usage`. Of these, `quota`, `402`, `locked`, `spend`,
`verify-timeout`, `bad-ref` and `empty-diff` never count against the model;
`no-edit`, `timeout` and `hang` do.

Verification: `implement` and `tests` runs get the full gates; only `grunt`
passes `--skip gates`. Verification has its own budget,
`--verify-timeout SEC` on `run` and `wave` (default 1800); when it expires
the run exits 2 with cause `verify-timeout`, a non-fault cause that `stats`
does not count, so a slow-but-passing draft is no longer reported as
`verify-error`. A PASS from the verifier with no changed file
(`files` empty) is not a PASS for `implement` and `tests`: the run exits 2,
with the infra cause when the event log holds an `error` event (`quota` for a
rate limit, `model` for any other `provider.*` error), else `no-edit`.
`grunt` gets the same `no-edit` rule from the slot instead of from `files`:
a grunt run with no change against `--base`, committed or uncommitted, exits
2 `no-edit` with `failed_checks` `["no-edit"]` (so it counts against the
model) and never reaches the verifier. `review` and `scout` are unaffected.

`crpg-grunt` has no `git add`/`git commit` permission, so `oc.py` commits a
verified grunt run itself: after a grunt PASS it stages exactly the files the
verifier reported in scope and makes one commit in the slot,
`<ID>: grunt edits (<model>)`, with no `--no-verify` and no amend. The
summary then carries a `committed <short hash>` line and `--json` a `commit`
key (the hash, or `null`). A failed or errored verification commits nothing
and leaves the files for the manager to inspect; `implement` and `tests`
commit their own work, and `oc.py` never commits for them. A failed grunt
commit exits 2 `verify-error`.

The summary shows the first 14 lines of the worker's report and a
`report: <path>` line; the whole report is in
`<repo>/.agents/runs/<ID>/<attempt>-<model>.report.md`, beside the event log
and the result record.

Tests live in `tools/tests/test_oc.py` and run against
`tools/tests/fake_opencode.py` (every failure mode above, both OpenCode
generations: `$FAKE_OC_VERSION` picks the `--version` answer, default
2.x; in 2.x mode the fake fails on `--dir`/`--variant` like the real
2.x) and
`tools/tests/fixtures/oc/fake_verify.py` (a stand-in for A0-4's verifier).
They use temporary git repositories and worktrees only — never the real
`opencode`, never the network — and finish in under 60 seconds:

```text
python -m unittest discover -s tools/tests -p "test_oc.py" -v
```

## Neighbours (other workers, not this script)

- `tools/agents/verify_task.py` — the verifier `oc.py` runs (A0-4). Faked in
  tests via `$CRPG_VERIFY`. Without `--brief` it uses the slot's `BRIEF.md`,
  unless that file's first line (`# Brief — <ID> ·` or `# Review brief — <ID> ·`)
  names a different task; then it uses `.agents/briefs/<ID>.md` in `$CRPG_REPO`
  (default: the repository holding the script), and exits 2 `no-brief` if
  neither exists. Slots 3–6 (directory basenames `slot-3` … `slot-6`) and any
  slot whose brief `Route:` line contains `doc-only` are doc-only: the gates
  never run cargo — with a crate owned they report `skipped (doc-only slot)`,
  and with no crate owned they run the tool unittests when `tools/`,
  `.opencode/` or `opencode.json` changed, else `skipped (no tool change)`.
  The Python lints still run, and the `doc-only` check fails on any changed
  `.rs`, `Cargo.toml` or `Cargo.lock`. Its `docsig` check accepts an added
  agent-log entry dated from the UTC day of the oldest commit in
  `base..HEAD` up to today (today only with no commits; `$CRPG_TODAY`
  overrides today).
- `tools/agents/brief.py`, `tools/agents/recall.py` — brief generation and
  memory search (A0-5). `oc.py` calls `brief.py` when `--brief` is omitted.

## Agent log

- 2026-10-16 (UTC) · opencode/muse-spark-1.3-contributor-free + A0-3 · Wrote this usage guide for the new `tools/agents` scripts so the manager can route OpenCode workers without reading `oc.py`; it ends with this signed entry per the root `AGENTS.md` rule.
- 2026-10-16 (UTC) · opencode/muse-spark-1.3-contributor-free + A0-3 attempt 3 · Documented the review fixes above (wave model caps, refused Zen IDs, brief precedence with placeholder and same-crate checks) so the guide matches `oc.py`'s routing behaviour.
- 2026-10-16 (UTC) · opencode/muse-spark-1.3-contributor + A0-3 attempt 4 · Documented OpenCode 2.x support (version probe, `--standalone` with model `#variant` and no `--dir`, kept 1.x form, removed Cat Jam IDs, top-level error events, versioned fake) so the guide matches `oc.py`'s dual-generation behaviour.
- 2026-10-16 (UTC) · claude-code/opus-5.5 + user: Beam is Cat Jam renamed · Recorded Beam Free as the Go ID behind both Cat Jam IDs and its own Zen ID, replacing the removed-model refusal, as the user directed.
- 2026-10-16 (UTC) · opencode-go/DeepSeek-Flash-V4.1 + A0-3b · Documented the `--base` pass-through to `verify_task.py` (run and wave) and the agent-declaration fallback with its `info/exclude` entries and `no-agent` cause, so the guide matches the slot preparation defect fixes.
- 2026-10-17 (UTC) · claude-code/sonnet-5.5 (implementer) + A0-3d · Documented the gates for the tests role, the empty-change `no-edit` rule, the report file, the brief-by-task-ID fallback and the docsig date range so the guide matches the five tool fixes.
- 2026-10-21 (UTC) · opencode-go/deepseek-v4.1-flash + A0-3c · Documented `spend`, the pre-start Go guard with its `spend` cause, the cap env vars and the override, so the manager can see the account-wide Go budget without a throwaway script.
- 2026-10-21 (UTC) · opencode-go/deepseek-v4.1-flash + A0-3f · Documented that `--base` splits a remote only when the slot configures it and that `run` checks out or creates a missing task branch (refusing uncommitted work), so the guide matches the session-10 tool fixes.
- 2026-10-23 (UTC) · opencode-go/deepseek-v4.1-flash + A0-3g · Documented the `no-edit` rule for grunt runs and the dispatcher's one commit of a verified grunt run (its `committed <hash>` line and `--json` `commit` key), so a grunt run that changed nothing can no longer print PASS and its work is no longer left uncommitted.
- 2026-10-23 (UTC) · opencode-go/deepseek-v4.1-flash + A0-4b · Documented the `--verify-timeout` budget and its non-fault `verify-timeout` cause, per-slug attempt counting with the slug file names, and the doc-only slot rule, so the guide matches the five session-14 tool fixes.
- 2026-10-23 (UTC) · opencode-go/deepseek-v4.1-flash + A0-4b attempt 3 · Corrected the doc-only gate rule: no cargo ever, but a tool change still runs the tool unittests (no tool change reports `skipped (no tool change)`), so a tools task's own tests are not silently skipped.
- 2026-10-23 (UTC) · opencode-go/deepseek-v4.1-flash + A0-4c · Documented `ask --diff/--base`, the diff file beside the run log, the exact question sentence, the recorded `diff` key, and the non-fault `bad-ref`/`empty-diff` exits, so a reviewer can read a slot branch's diff from the main checkout without loosening a permission.
