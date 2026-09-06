## Task
Correct the spec's wall-clock event-graph budget that breaks determinism.
Human-decision task (spec edit).

## Why this is deferred / blocked

Spec §5.2 (determinism; section citation corrected per E009 2026-09-06 — was "§4") — the reviewer flagged §526–531: the event graph budget is
described as (some form of) *wall-clock* time, and when a graph exceeds the
budget it is aborted. Wall-clock time is nondeterministic; an abort keyed to it
produces different behavior across machines/runs, which violates the
determinism invariant that the whole sim exists to preserve. The correct budget
is an *instruction* budget (the Lua VM spike already uses one).

## Decision to make
- Change the spec wording from "wall-clock budget" to "instruction/bytecode
  budget" for event-graph expansion, matching the existing Lua instruction
  budget pattern.
- Keep "abort on budget exhaustion" semantics (the abort is fine; the clock used
  to measure it is what must be deterministic).

## Deliverable
- Edit `docs/CRPG_ENGINE_SPEC.md` §526–531 to measure event-graph expansion in
  instruction count, not wall-clock time.
- Note the change with a dated line so reviewers can see the correction.

## Constraints
- Do not edit crpg sources in this task.
- Spec edit: human sign-off required.

## Agent log

- 2026-09-05 (UTC) · opencode/muse-spark · Applied per maintainer sign-off: §5.2 budget is instruction/bytecode; see spec Agent log.
- 2026-09-06 (UTC) · opencode/muse-spark + E009 · Fixed the section citation (§4 → §5.2); no decision change.