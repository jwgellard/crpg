# Bake-off harness

A tool to re-run and re-score the model bake-off. Fixtures are data: one
directory each under `fixtures/`. One command re-runs and re-scores every
configured model.

## What a fixture is

Each `fixtures/<name>/` contains a `fixture.json` plus the fixture's files.
`fixture.json` keys:

- `kind`: `scout` | `implement` | `review`.
- `prompt`: the prompt text.
- `timeout_s`: per-run timeout.
- `edit`: whether the worker may edit (only implement).
- `files`: how the template is built. Entries may be `{"tree": "<dir>"}`,
  `{"repo": "<path>", "dest": "<path>"}`, `{"commit": "<sha>", "paths": [...],
  "dest": "<dir>"}`, or `{"rename": {"<from>": "<to>"}}`.
- `hidden`: files copied into the run directory only at scoring time, kept under
  `<scratch>/ref/`.
- `score`: by kind. Scout: `truth` (sorted JSON array). Implement: `gates`
  (each with `cmd`, `expected`, optional `name`), `hidden_test`, `hidden_total`,
  `rules`. Review: `file` (the file under review, required; the edit check
  compares it against the template, and `setup` and `score` refuse a review
  fixture whose `file` is missing or is not in the template), `defects` (`id`,
  `lines` [lo, hi], `keywords`
  regex or null), `decoy_lines` (a bare int for one line, a range `[lo, hi]`,
  or a list mixing lines and ranges `[29, [lo, hi], ...]`; ranges are
  inclusive), `decoy_specs`.

Rules files and specs are stored as `.txt` and renamed to `.md` at setup, so no
agent in this repository loads them as instructions.

## Adding a fixture

Create `fixtures/<name>/fixture.json` and its files. No harness change needed:
the setup, run and score commands discover fixtures automatically.

## Running

```
python tools/agents/bakeoff/bakeoff.py setup <scratch>
python tools/agents/bakeoff/bakeoff.py run <scratch> --fixtures all --models @tools/agents/bakeoff/models.txt
python tools/agents/bakeoff/bakeoff.py score <scratch>
python tools/agents/bakeoff/bakeoff.py all <scratch>
```

Keep `<scratch>` outside the repository.

## Fixtures

- `scout-deps` (scout): which crates may depend on `crpg-sim`, from `tools/lint/deps.py` at `16f58c8`.
- `implement-fixed-point` (implement): three `todo!()` functions from local files, hidden tests in `reference.rs.txt`.
- `review-planted` (review): synthetic defects in `src/combat.rs`, no source commit.
- `review-t019` (review): T019 same-pool affordability, `crates/crpg-sim/src/combat.rs` at `56931eb^`; 28-line verbatim excerpt (lines 1579-1606; the full file is 2041 lines).
- `review-t129` (review): T129 token-bucket watermark, `crates/crpg-net/src/sim.rs` at `1cc34a6^` (598 lines).
- `review-t130` (review): T130 zero `OnTick` period on decode, `crates/crpg-rules/src/resource.rs` at `caad63f^` (419 lines).
- `review-t131` (review): T131 capture history gaps, `crates/crpg-server/src/checkpoint.rs` at `bdc6aec^` (500 lines).
- `review-t023d` not built: the close-delivery fix arrived inside the squashed PR #28 import (`16f58c8`), so no pre-fix blob exists in this history.

## Agent log

- 2026-10-21 (UTC) · opencode-go/beam-free + A0-7a · Promoted the one-off bake-off harness to a tool with data-driven fixtures.
- 2026-10-21 (UTC) · opencode-go/beam-free + A0-7a attempt 2 · Scorer now runs fixture gates and enforces the fixture's unchanged-files list; decoy range made inclusive via nested ranges.
- 2026-10-21 (UTC) · opencode-go/mimo-v2.6-pro + A0-7a attempt 3 · Review fixes: the float rule's regex actually matches `f32`/`f64`, table display and pass counts come from fixture data, `decoy_lines` accepts bare int / flat range / nested ranges, and `score` refuses a scratch inside the repository.
- 2026-10-21 (UTC) · opencode-go/mimo-v2.6-pro + A0-7a attempt 4 · The review scorer's edit check now follows the fixture's `score.file` instead of a hard-coded `src/combat.rs`; `setup` and `score` refuse a review fixture whose `score.file` is missing or is not in the template.
- 2026-10-22 (UTC) · opencode-go/muse-spark-1.3-contributor + A0-7b · Added review fixtures cut from four real fixes (T019 as a 28-line excerpt, T129/T130/T131 as full pre-fix files) with the all-fixtures setup test; review-t023d skipped because its pre-fix blob is absent from this history.