# Bake-off, 2026-10-16

The evidence behind `docs/agents/MODELS.md` §§3–5: the harness, the three
fixtures and the scored results of 26 OpenCode models, kept so setup task A0-7
can promote the harness to `tools/agents/bakeoff/` and re-run it. This
directory is a record, not a tool: nothing in CI runs it.

| Path | What it is |
|---|---|
| `harness/setup_fixtures.py` | Builds `<scratch>/tpl/{t1,t2,t3}` and `<scratch>/ref/hidden.rs` from the fixtures and the repository's `AGENTS.md` and `tools/lint/deps.py` |
| `harness/bake.py` | Runs `opencode run` per model and task in an isolated copy, with the fixes from `MODELS.md` §1 |
| `harness/score.py` | Scores the runs: scout answer, hidden tests and gates, planted bugs |
| `harness/table.py` | Merges three scored runs into one Markdown table (expects the session's `bake`, `bake2`, `bake3` layout) |
| `fixtures/implement/` | The `todo!()` crate. `AGENTS.txt` becomes `AGENTS.md` at setup; `hidden.rs.txt` holds the 21 hidden tests and `reference.rs.txt` the reference solution that passes them |
| `fixtures/review/` | The planted-defect module; `SPEC.txt` becomes `SPEC.md` at setup |
| `results/t1.json`, `t2.json`, `t3.json` | Scores for scout, implement and review, with paths removed |

The fixture rules files and spec are stored as `.txt` so no agent working in
this repository loads them as instructions.

Re-run, from Git Bash, with a scratch directory **outside** the repository:

```bash
python -I docs/agents/bakeoff-2026-10-16/harness/setup_fixtures.py . "$SCRATCH"
cd "$SCRATCH/tpl" && BAKE_PAR=4 python -I <repo>/docs/agents/bakeoff-2026-10-16/harness/bake.py "$SCRATCH" t1,t2,t3 opencode-go/deepseek-v4.1-flash ...
python -I <repo>/docs/agents/bakeoff-2026-10-16/harness/score.py "$SCRATCH"
```

`t1` was run against `AGENTS.md` and `tools/lint/deps.py` at `16f58c8`; a later
`ALLOWED` table changes its expected answer (`TRUTH1` in `score.py`).

---

## Agent log

- 2026-10-16 (UTC) · claude-code + agent-team plan bake-off · Saved the harness, fixtures and path-free results of the same-day bake-off so the routing table's evidence survives the session and A0-7 can promote and re-run it.
