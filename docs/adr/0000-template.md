# ADR-NNNN: <PLACEHOLDER — short decision title>

> **This is a template, not a decision.** Every `<PLACEHOLDER …>` below must be
> replaced. A copied template with placeholders left in is not an ADR, has no
> status, and records no approval or test result. Number new ADRs with the next
> free four-digit number; never reuse or renumber one.

Date: <PLACEHOLDER — YYYY-MM-DD (UTC) the ADR was filed>
Status: <PLACEHOLDER — exactly one of the statuses below, with its authority>

Status vocabulary (E007 policy, T032):

- **Proposed** — filed for review; nothing is approved yet.
- **Selected under delegation** — chosen by an agent exercising a recorded
  delegation (name the record, e.g. `tasks/POST-T018-DECISIONS.md` D08). Not a
  claim of independent human review.
- **Accepted** — approved by a named human decision; cite where it is recorded
  (task file, PR, decision record). Never write Accepted for yourself, and never
  backdate an acceptance.
- **Superseded by ADR-MMMM** (optionally "in part: <scope>") — a later ADR
  replaced this decision; the text below stays as it was decided.

## Context

<PLACEHOLDER — the forces and facts that make a decision necessary now. Link
the task, review or incident that raised it. State what is true today, not
what the decision will make true.>

## Decision

<PLACEHOLDER — the decision itself, in the imperative. Once the status is
Selected or Accepted this section is not edited: corrections and supersession
are appended below or made by a new ADR.>

## Authority and affected surface

- Decided by: <PLACEHOLDER — human name/role, or the delegation record>
- Authoritative contract: <PLACEHOLDER — the exact public API items, or the
  task file section that pins them (for example `tasks/TNNN.md#specification-revision`).
  Point at one owner; do not keep a second competing copy here.>
- Owning crate(s)/files: <PLACEHOLDER>

## Compatibility, hash and dependency impact

- Persisted/serialized shapes: <PLACEHOLDER — unchanged, or exactly what changes
  and the migration/absence strategy>
- Replay, `state_hash`, goldens: <PLACEHOLDER — unchanged, or which new goldens
  and why; existing goldens are never rebaselined silently>
- Dependency graph (`tools/lint/deps.py` `ALLOWED`) and external dependencies:
  <PLACEHOLDER — unchanged, or the separate approval record>
- Platform scope (ADR-0012): <PLACEHOLDER>

## Alternatives considered

<PLACEHOLDER — each rejected option and the concrete reason it lost.>

## Consequences

<PLACEHOLDER — what becomes easier, what becomes harder, what is deliberately
left open and who owns it.>

## Acceptance and evidence

<PLACEHOLDER — the commands/tests that will show the decision is implemented,
and where their results will be recorded (normally the task's completion
record). Until results exist, write "not yet run". Do not write "tests
passed" in a template or before the run happened, on which native target.>

## Supersession and corrections

<PLACEHOLDER — leave empty at filing. Later: append dated entries such as
"YYYY-MM-DD (UTC): superseded in part by ADR-MMMM (scope)" or a factual
correction note. Never rewrite the Decision section to match a later choice.>

## Agent log

<PLACEHOLDER — every agent edit appends one line in the mandatory format:
`YYYY-MM-DD (UTC) · <harness/model> + <task or reason> · 1–2 sentence why`.
Human edits need no entry; never forge a human entry or sign-off.>

- 2026-09-30 (UTC) · claude-code + T032 ADR template · Added the ADR template with the E007 status vocabulary and placeholder-only acceptance text, so a copied template cannot read as an accepted decision or a passed test.
