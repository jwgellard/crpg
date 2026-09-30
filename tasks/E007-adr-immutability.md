## Task
Clarify the ADR immutability rule after T002b edited ADR-0004. Human-decision task.

## Why this is deferred

`docs/architecture/README.md` says ADRs are "created for decisions; records are
immutable — never edited". But the review found T002b appended/amended ADR-0004
during its work. The project also relies on dated, dated appendices (ADR-0007
sets a new policy rather than editing the old decision).

## Decision to make
- Adopt an explicit rule, e.g.: *the "Decision" section is immutable; evidence,
  corrections, and links may be appended dated*. Record it in
  `docs/architecture/README.md`.
- Optionally provide a short "ADR lifecycle" note: new decision supersedes old
  (new ADR) vs edit-in-place (correction), with guidance.

## Deliverable
- One paragraph or bullet list in `docs/architecture/README.md` defining the
  immutable-vs-append/fix boundary.

## Constraints
- Doc-only; no source or lint changes. Human sign-off so the docs rule is
  stable for future tasks.
## Resolution — 2026-09-30 (T032, POST-T018 D18)

ADR correction policy, as selected under the recorded delegation:

- **The Decision section of an ADR is never rewritten** once its status is
  Selected or Accepted. To change a decision, file a **new numbered ADR** that
  links its predecessor and states the scope it supersedes (whole or "in
  part"), and append to the predecessor a dated line such as
  `YYYY-MM-DD (UTC): superseded in part by ADR-MMMM (scope)`.
- **Evidence, factual corrections and links may be appended**, dated, in a
  clearly separate section (as ADR-0004 gained T002b's evidence and ADR-0012
  supersedes only ADR-0009 Decision 3). Previous decisions, status provenance
  and agent logs are preserved verbatim.
- **Status vocabulary is exact:** *Proposed* (filed, nothing approved);
  *Selected under delegation* (chosen by an agent exercising a named delegation
  record — not independent human review); *Accepted* (approved by a named human
  decision, cited); *Superseded by ADR-MMMM*. Never backdate an approval or
  promote Selected to Accepted without a recorded human decision.

The policy is embodied in `docs/adr/0000-template.md` (T032) and in the spec
§14 tree note. `docs/architecture/README.md`, which this task originally
named as the deliverable location, is outside T032's allowed files; pointing
it at this resolution is a small follow-up, not a policy gap.

- 2026-09-30 (UTC) · claude-code + T032 E007 resolution · Stated the append-only supersession policy and exact status vocabulary, and noted that the architecture README pointer is a follow-up outside T032's scope.
