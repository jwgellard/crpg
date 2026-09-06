## Task
Fix the layer diagram's dependency direction and the double meaning of
"core". Human-decision task (spec + README edit).

## Why this is deferred

- The §2.2 diagram stacks `crpg-net` above `crpg-sim` above
  `crpg-ai | crpg-script | crpg-persist` above `crpg-rules`
  (`docs/CRPG_ENGINE_SPEC.md:154-166`) under "dependency direction is
  strictly downward" (`:169`). Read literally, `sim` depends on
  `ai/script/persist`. The enforced rule is the reverse:
  `core <- data <- rules <- sim <- {net, ai, script} <- server` (root
  `AGENTS.md` spine; the `ALLOWED` table in `tools/lint/deps.py` is normative
  per `README.md:126-127`). `README.md:32-37` repeats the ambiguous stacking,
  so the only unambiguous sources are the spine sentence and the lint table.
- "Core" means the whole workspace in §0 (`docs/CRPG_ENGINE_SPEC.md:18-19`:
  "`crpg-core` ← rules, world state, simulation, …") and the bottom crate in
  §2. A reader cannot tell which "core" a requirement constrains.

## Decision to make
- Redraw or annotate the diagram so downward matches the enforced spine
  (or state the diagram is data-flow, not dependency, if that was the
  intent); reserve bare "core" for the bottom crate and rename the §0
  umbrella ("workspace"/"simulation core").

## Deliverable
- Edits to `docs/CRPG_ENGINE_SPEC.md` (§§0, 2.2) and `README.md:32-37`,
  each with a dated inline note. No source changes.

## Constraints
- Doc-only. Human sign-off; the diagram is what new agents read before the
  lint table.

## Agent log

- 2026-09-06 (UTC) · opencode/muse-spark · Filed as part of the spec-gap triage: the diagram and the lint table point in opposite directions.
