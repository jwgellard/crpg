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
                               [--timeout SEC] [--variant V] [--brief PATH]
                               [--notes PATH] [--slug SLUG] [--base REF]
                               [--fresh] [--dry-run] [--json] [--full]
python tools/agents/oc.py wave SPEC [SPEC ...] [--timeout SEC] [--base REF] [--json]
python tools/agents/oc.py ask  --role scout --model MODEL [--timeout SEC] [--json] QUESTION
python tools/agents/oc.py record --task ID --role ROLE --model MODEL --outcome OUTCOME
                               [--attempt N] [--note TEXT]
python tools/agents/oc.py stats [--role ROLE] [--since YYYY-MM-DD] [--json]
```

`SPEC` is `ID:ROLE:MODEL`; `ROLE` is `implement`, `tests`, `review`,
`scout` or `grunt`, and the worker runs as the `crpg-<role>` agent.
`OUTCOME` is `review-ready`, `review-fix`, `review-reject`, `merged` or
`reverted`. Slots are warm git worktrees under `$CRPG_SLOTS` (default
`<repo>/../crpg-slots`); without `--slot`, the lowest-numbered free slot
wins. Every `run`, `ask` and `wave` member leaves a result record under
`<repo>/.agents/runs/<ID>/` (the model scorecard `stats` ranks) and an
event log beside it.

Environment: `$CRPG_REPO` (repo root, default: the main worktree),
`$CRPG_SLOTS`, `$CRPG_OPENCODE` (executable; a `.py` path runs under
`sys.executable`), `$CRPG_VERIFY` (verifier path), `$CRPG_OPENCODE_LOG`
(OpenCode's own log, for the idle watchdog), `$CRPG_OC_START_GAP` (worker
start spacing, default 4 s) and `$CRPG_OC_LOCK_WAIT` (locked retry wait,
default 5 s).

Exit-2 causes: `fetch`, `no-brief`, `no-verify`, `no-slot`, `slot-busy`,
`no-exe`, `branch`, `locked`, `402`, `quota`, `timeout`, `hang`,
`no-edit`, `model`, `verify-error`, `usage`. Of these, `quota`, `402` and
`locked` never count against the model; `no-edit`, `timeout` and `hang` do.

Tests live in `tools/tests/test_oc.py` and run against
`tools/tests/fake_opencode.py` (every failure mode above) and
`tools/tests/fixtures/oc/fake_verify.py` (a stand-in for A0-4's verifier).
They use temporary git repositories and worktrees only — never the real
`opencode`, never the network — and finish in under 60 seconds:

```text
python -m unittest discover -s tools/tests -p "test_oc.py" -v
```

## Neighbours (other workers, not this script)

- `tools/agents/verify_task.py` — the verifier `oc.py` runs (A0-4). Faked in
  tests via `$CRPG_VERIFY`.
- `tools/agents/brief.py`, `tools/agents/recall.py` — brief generation and
  memory search (A0-5). `oc.py` calls `brief.py` when `--brief` is omitted.

## Agent log

- 2026-10-16 (UTC) · opencode/muse-spark-1.3-contributor-free + A0-3 · Wrote this usage guide for the new `tools/agents` scripts so the manager can route OpenCode workers without reading `oc.py`; it ends with this signed entry per the root `AGENTS.md` rule.
