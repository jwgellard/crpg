# Authoring benchmark

A benchmark for agent campaign authoring: five briefs of rising size are given
to several models inside a bare directory holding only the `crpgc` command, the
23 JSON Schema files and a list of today's commands; each run is scored on
validity, completeness, playability, steps and errors. The same tool re-runs on
each candidate command design and compares it to the baseline, so T101's
authoring-UX ADR has a number to beat.

The command line (a `<scratch>` is any directory outside the repository):

```
python tools/agents/authorbench/authorbench.py setup <scratch>
python tools/agents/authorbench/authorbench.py run <scratch> --models <m1,m2|@file>
        [--briefs <all|a,b>] [--design NAME] [--out NAME] [--variant V]
        [--repeat N] [--par N] [--crpgc PATH] [--tools PATH]
python tools/agents/authorbench/authorbench.py score <scratch>
        [--design NAME] [--out NAME] [--crpgc PATH] [--json]
python tools/agents/authorbench/authorbench.py compare <scratch> <base-out> <cand-out> [--json]
python tools/agents/authorbench/authorbench.py all <scratch> [--models ...] [the run flags]
```

`all` runs `setup`, then `run` (defaulting `--models` to
`@tools/agents/authorbench/models.txt`), then `score`.

## What it measures

Four score fields, recorded separately and never combined into one number:

- `valid` — `crpgc validate campaign --json` exits 0 and prints a JSON array.
- `complete` — the documents the brief asked for exist (the `minimum` floor,
  and a ruleset brief's `ruleset_slug`).
- `playable` — `crpgc play --campaign campaign` (fed 400 lines of `1`) starts
  the first fight with at least the brief's number of combatants. `finished`
  (a summary printed) is recorded, not required.
- `steps` / `errors` — from the model's event log: `step_finish` events, and
  `bash` tool calls that ran `crpgc` and failed.

A row also records `files`, `kinds`, `validate` (errors/warnings/codes),
`play` (started/finished/combatants/rounds), `first_error`, the token and cost
totals, and a scrubbed `err`. The score table (`cells`) counts and averages
these per brief and model.

## A brief

A brief is a directory `briefs/<name>/` with a `brief.json` (the KEYS: name,
kind, starter, minimum, ruleset_slug, min_combatants, timeout_s) and a
`prompt.txt` whose `{{TOOLS}}` mark is replaced by the tool list at run time.
`setup` builds one template per brief under `<scratch>/tpl/<name>/`: the
prompt as `brief.md`, a copy of the repository's `schemas/*.json`, and, for the
extend brief, the starter campaign. Five briefs ship, in size order:
`campaign-extend`, `campaign-small`, `campaign-medium`, `campaign-large`,
`ruleset-small`.

## A design

A design is a label, a binary and a tool list. The baseline is `crpgc` as
built today with `tools-baseline.txt`. Run a candidate with
`--design NAME --crpgc <candidate build> --tools <candidate tool list>`, then
`compare <scratch> baseline NAME`; a `--tools` path is relative to the current
directory unless absolute. `--design` labels the run (default `baseline`);
`--out` names the results directory (default: the design). `compare` reads two
`<scratch>/<out>/scores.json` files and reports, per brief and model, `better`,
`same` or `worse` by playable, then complete, then valid; step and error means
are shown as differences and never decide.

## scores.json

`<scratch>/<out>/scores.json` holds `tool`, `version`, `design`, `smoke`
(the `crpgc run` health check), `rows` (one per run) and `cells` (one per
brief/model). Run metadata lives in `<scratch>/<out>/logs/*.jsonl.meta.json`;
`dir` and `log` are relative to `<out>/`.

## Privacy

The repository is public and free models may train on prompts, so every byte
given to a model is repository content (the prompts, the tool list, the
schemas, the starter campaign). Scores hold model ids, counts and `crpgc`
diagnostics only: no absolute path, no prompt, no model output; `err` is
scrubbed of the scratch and home paths.

## Limits

Only combat can be played today: a large brief's item, dialogue and quest are
validated and counted, never played. The agent is told no folder layout (D72b);
a re-run with another `--tools` file is how a layout hint is tried.

## Baseline

```
python tools/agents/authorbench/authorbench.py setup <scratch>
python tools/agents/authorbench/authorbench.py run <scratch> --models @tools/agents/authorbench/models.txt --design baseline --repeat 2 --par 3 --crpgc target/release/crpgc.exe
python tools/agents/authorbench/authorbench.py score <scratch> --design baseline
```

## Agent log

- 2026-11-20 (UTC) · opencode-go/deepseek-v4-pro + T100b · Wrote the command line, the five briefs, the tool and model lists and this README so the authoring benchmark can be run and its exact-valued suite can pin the contract's data.
