## Task
Record the determinism scope ("replay, not lockstep") in an ADR, as the spec
itself demands. Human-decision task (new ADR).

## Why this is deferred

`docs/CRPG_ENGINE_SPEC.md:201` defines the project's central invariant —
replay determinism mandatory (same binary, same inputs, same result),
cross-platform lockstep explicitly not required — and ends: "This … should be
stated in an ADR." No ADR states it (checked: 0001, 0002, 0006, 0007, 0008).
`README.md:82-84` repeats the formula without an ADR citation. T008
(`state_hash` + tick loop) and T009 (replay harness) need the scope as an
acceptance criterion: without it, a hash divergence across platforms reads as
a bug rather than out-of-scope, and `state_hash`'s exclusion set ("excludes
presentation-only and non-deterministic fields," spec `:1160`) has no
authority behind it.

## Decision to make
- Adopt the scope as an ADR (new number): exact formula, what
  cross-platform sameness is *not* promised, and what `state_hash` may
  exclude. Confirm the server-authoritative justification that makes lockstep
  unnecessary.

## Deliverable
- New `docs/adr/NNNN-*.md` + one-line citations from spec §2.4 and T008/T009
  rows (read T008a/T008b after the E004 split, 2026-09-06). No source changes.

## Constraints
- Doc-only. Human sign-off; this ADR will be cited by every determinism
  dispute for the life of the project.

## Agent log

- 2026-09-06 (UTC) · opencode/muse-spark · Filed as part of the spec-gap triage: the spec orders this ADR itself and it was never written.
- 2026-09-06 (UTC) · opencode/muse-spark + E004 split · The `state_hash` acceptance-criterion need sits in T008a now; BACKLOG Blocks retargeted from T008 to T008a accordingly. T009 still needs it too.
